#!/usr/bin/env python3
"""End-to-end test of the analysis server + UI pair.

Usage: e2e_analysis_ui.py <crabbit-analysisd> <pliron-inspect> <module.plir>

Starts crabbit-analysisd (HTTP shim), drives real runs (baseline and
eregalloc-c2+spectral) over the wire protocol, exercises per-pass IR,
ir_hover/ir_definition/ir_references/ir_diagnostics, artifact download
(ELF magic checked); then starts the pliron-inspect UI server pointed at
it and asserts the frontend serves and a full run works through the UI's
own /api/analysis route. Exits non-zero on any failure.
"""
import json, re, socket, subprocess, sys, time, urllib.request

analysisd, inspect_bin, plir = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(plir).read()
fails = []

def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

def check(name, cond, detail=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  {detail}" if detail and not cond else ""))
    if not cond: fails.append(name)

aport = free_port()
ad = subprocess.Popen([analysisd, "--workers", "2", "--http", f"127.0.0.1:{aport}"],
                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
def http(addr, path, body=None, timeout=600):
    req = urllib.request.Request(f"http://{addr}{path}",
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json"}, method="GET" if body is None else "POST")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()
def api(cmd, timeout=600):
    st, body = http(f"127.0.0.1:{aport}", "/", cmd, timeout)
    return json.loads(body)

for _ in range(50):
    try:
        if api({"cmd": "server_health"}).get("ok"): break
    except Exception: time.sleep(0.2)
else:
    sys.exit("analysisd HTTP never came up")

h = api({"cmd": "server_health"})
check("server_health", h.get("ok") is True)
targets = api({"cmd": "list_targets"}).get("targets", [])
check("list_targets has aarch64-linux", "aarch64-linux" in targets, str(targets))

m = api({"cmd": "load_module", "name": "e2e", "text": text})
check("load_module", "moduleId" in m, str(m)[:200])
mid = m.get("moduleId")

progress_seen = []

def run(config, inspect):
    r = api({"cmd": "start_run", "moduleId": mid, "target": "aarch64-linux",
             "config": config, "inspect": inspect})
    check(f"start_run {config}", "runId" in r, str(r)[:200])
    rid, passes = r.get("runId"), r.get("passes", [])
    saw_partial = False
    for _ in range(6000):
        st = api({"cmd": "run_status", "runId": rid})
        done = len(st.get("passes", []))
        if st.get("status") in ("done", "failed", "cancelled"): break
        if 0 < done < st.get("totalPasses", 0): saw_partial = True
        time.sleep(0.05)
    check(f"run {rid} done", st.get("status") == "done", str(st)[:300])
    progress_seen.append(saw_partial)
    return rid, passes

rid_base, passes = run({}, inspect=True)
rid_ereg, _ = run({"CRABBIT_REGALLOC": "eregalloc", "CRABBIT_REGALLOC_ORACLE": "c2",
                   "CRABBIT_BLOCK_FREQ": "spectral"}, inspect=False)

# A fast run can finish between polls; partial progress must be observable
# on at least one run (the inspect run renders per-pass IR and is slower).
check("partial progress observed on some run", any(progress_seen))

ir_early = api({"cmd": "run_ir", "runId": rid_base, "pass": 0})
check("run_ir early (captured)", ir_early.get("source") == "captured" and "builtin.module" in ir_early.get("ir",""))
ir_final = api({"cmd": "run_ir", "runId": rid_base})
check("run_ir final", "ir" in ir_final, str(ir_final)[:200])
ir_final_replay = api({"cmd": "run_ir", "runId": rid_ereg})
check("run_ir final (replay path)", ir_final_replay.get("source") == "replay" and "ir" in ir_final_replay)

# Backward cost lift (run_costs): a synthetic op_costs payload against the
# real attribution boundary. Source id 0 is the function's first op by
# construction of the head stamping; roots pass through to `roots`.
sym_m = re.search(r'llvm\.func @([A-Za-z0-9_$.]+)', text)
if sym_m:
    symbol = sym_m.group(1)
    payload = {symbol: {"machine": {"0": 4},
                        "lifted": {"0": 10, "frame": 4},
                        "source": {"0": 10.0, "frame": 4.0}}}
    rc = api({"cmd": "run_costs", "runId": rid_base, "opCosts": payload})
    check("run_costs answers", rc.get("level") == "source" and "ir" in rc, str(rc)[:300])
    fns = {f.get("symbol"): f for f in rc.get("functions", [])}
    fc = fns.get(symbol, {})
    entries = {e.get("id"): e for e in fc.get("entries", [])}
    check("run_costs maps id 0 with cost", entries.get("0", {}).get("cost") == 10.0,
          str(fc)[:300])
    check("run_costs entry has snippet", bool(entries.get("0", {}).get("snippet")),
          str(entries.get("0"))[:200])
    check("run_costs roots accounting", fc.get("roots", {}).get("frame") == 4.0,
          str(fc.get("roots"))[:200])
    rc_ra = api({"cmd": "run_costs", "runId": rid_base, "opCosts": payload, "level": "ra"})
    check("run_costs ra level", rc_ra.get("level") == "ra" and "functions" in rc_ra,
          str(rc_ra)[:200])
else:
    check("run_costs symbol found in module", False, "no llvm.func in fixture")

# Language features on the loaded module: find a value def and a later use.
lines = text.splitlines()
mdef = re.search(r"^\s*(v\d+) =", text, re.M)
name = mdef.group(1)
use = None
for i, l in enumerate(lines):
    for mm in re.finditer(rf"\b{name}\b", l):
        if not re.match(rf"^\s*{name} =", l):
            use = (i, mm.start())
    if use: break
def_line = next(i for i, l in enumerate(lines) if re.match(rf"^\s*{name} =", l))
hov = api({"cmd": "ir_hover", "moduleId": mid, "position": {"line": use[0], "character": use[1]+1}})
check("ir_hover has typed contents", bool(hov.get("contents")) and ":" in (hov.get("contents") or ""), str(hov)[:200])
dfn = api({"cmd": "ir_definition", "moduleId": mid, "position": {"line": use[0], "character": use[1]+1}})
check("ir_definition points at def line", dfn.get("range", {}).get("start", {}).get("line") == def_line, f"{dfn} expected line {def_line}")
refs = api({"cmd": "ir_references", "moduleId": mid, "position": {"line": use[0], "character": use[1]+1}})
check("ir_references >= 2 (def+use)", len(refs.get("ranges", [])) >= 2, str(refs)[:200])
diag = api({"cmd": "ir_diagnostics", "moduleId": mid})
check("ir_diagnostics parsed", diag.get("parsed") is True, str(diag)[:300])
# Language features on a RUN's rendered IR (early pass, still llvm dialect).
hov2 = api({"cmd": "ir_hover", "runId": rid_base, "pass": 0,
            "position": {"line": use[0], "character": use[1]+1}})
check("ir_hover on run IR answers", "error" not in hov2, str(hov2)[:200])

art = api({"cmd": "run_artifact", "runId": rid_base})
check("artifact present", "artifactBase64" in art, str(art)[:200])
if "artifactBase64" in art:
    import base64
    obj = base64.b64decode(art["artifactBase64"])
    check("artifact is ELF", obj[:4] == b"\x7fELF", str(obj[:8]))

# ---- UI server ----
uport = free_port()
ui = subprocess.Popen([inspect_bin, "--port", str(uport), "--no-open",
                       "--server", f"127.0.0.1:{aport}"],
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
uaddr = f"127.0.0.1:{uport}"
for _ in range(50):
    try:
        st, body = http(uaddr, "/"); break
    except Exception: time.sleep(0.2)
else:
    sys.exit("UI server never came up")
check("UI serves frontend", st == 200 and b"Start run" in body, body[:120])
st, body = http(uaddr, "/api/analysis/health")
check("UI analysis health proxy", st == 200 and json.loads(body).get("ok") is True, body[:200])

def uapi(cmd):
    st, body = http(uaddr, "/api/analysis", cmd)
    return json.loads(body)
m2 = uapi({"cmd": "load_module", "name": "via-ui", "text": text})
check("UI load_module", "moduleId" in m2, str(m2)[:200])
r2 = uapi({"cmd": "start_run", "moduleId": m2.get("moduleId"), "target": "aarch64-linux",
           "config": {}, "inspect": False})
check("UI start_run", "runId" in r2, str(r2)[:200])
for _ in range(6000):
    st2 = uapi({"cmd": "run_status", "runId": r2.get("runId")})
    if st2.get("status") in ("done", "failed", "cancelled"): break
    time.sleep(0.05)
check("UI-driven run done", st2.get("status") == "done", str(st2)[:300])
a2 = uapi({"cmd": "run_artifact", "runId": r2.get("runId")})
check("UI artifact", "artifactBase64" in a2, str(a2)[:200])

ui.terminate(); ad.stdin.close(); ad.terminate()
print(f"\n{'ALL PASS' if not fails else 'FAILURES: ' + ', '.join(fails)} ({0 if not fails else len(fails)} failed)")
sys.exit(1 if fails else 0)
