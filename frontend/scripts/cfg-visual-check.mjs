#!/usr/bin/env node

import { readFile, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { instance } from "@viz-js/viz";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const artifactsDir = path.join(root, "artifacts", "cfg-visual");
const args = parseArgs(process.argv.slice(2));
const screenshotPath = path.resolve(args.screenshot ?? path.join(artifactsDir, "cfg-dot.png"));
const metricsPath = path.resolve(args.metrics ?? path.join(artifactsDir, "cfg-dot-metrics.json"));
const dot = args.dot ? await readFile(path.resolve(args.dot), "utf8") : defaultCfgDot();

let chromium;
try {
  ({ chromium } = await import("@playwright/test"));
} catch {
  console.error("Missing @playwright/test. Install it with: npm install -D @playwright/test");
  console.error("Then install a browser with: npx playwright install chromium");
  process.exit(2);
}

await mkdir(path.dirname(screenshotPath), { recursive: true });
await mkdir(path.dirname(metricsPath), { recursive: true });

const viz = await instance();
const svg = viz.renderString(dot, { format: "svg", engine: "dot" });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1800, height: 1200 }, deviceScaleFactor: 1 });

try {
  await page.setContent(renderPage(svg), { waitUntil: "load" });
  await page.locator(".canvas").screenshot({ path: screenshotPath });
  const metrics = await page.evaluate(() => {
    const shrinkRect = (rect, amount) => ({
      left: rect.left + amount,
      right: rect.right - amount,
      top: rect.top + amount,
      bottom: rect.bottom - amount
    });

    const contains = (rect, point) =>
      point.x >= rect.left && point.x <= rect.right && point.y >= rect.top && point.y <= rect.bottom;

    const nodes = Array.from(document.querySelectorAll("g.node"))
      .map((node) => {
        const rect = node.getBoundingClientRect();
        return {
          id: node.querySelector("title")?.textContent ?? "",
          rect: shrinkRect(rect, 5)
        };
      });

    const intersections = [];
    const edges = Array.from(document.querySelectorAll("g.edge")).map((edge) => {
      const id = edge.querySelector("title")?.textContent ?? "";
      const paths = Array.from(edge.querySelectorAll("path")).filter((path) => path instanceof SVGPathElement);
      let sampled = 0;
      for (const path of paths) {
        const matrix = path.getScreenCTM();
        const length = path.getTotalLength();
        if (!matrix || length <= 0) {
          continue;
        }
        for (let step = 3; step < 97; step += 2) {
          const rawPoint = path.getPointAtLength((length * step) / 100);
          const point = new DOMPoint(rawPoint.x, rawPoint.y).matrixTransform(matrix);
          sampled += 1;
          for (const node of nodes) {
            if (contains(node.rect, point)) {
              intersections.push({ edgeId: id, nodeId: node.id, x: point.x, y: point.y });
              break;
            }
          }
        }
      }
      return { id, sampled };
    });

    return {
      nodeCount: nodes.length,
      edgeCount: edges.length,
      intersections,
      edges
    };
  });

  const payload = {
    renderer: "Graphviz dot via @viz-js/viz",
    screenshotPath,
    metricsPath,
    ...metrics
  };
  await writeFile(metricsPath, JSON.stringify(payload, null, 2));
  console.log(JSON.stringify(payload, null, 2));
  if (metrics.intersections.length > 0) {
    process.exitCode = 1;
  }
} finally {
  await browser.close();
}

function renderPage(svg) {
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <style>
      html, body {
        margin: 0;
        min-width: 1800px;
        min-height: 1200px;
        background: #111418;
      }
      .canvas {
        width: 1800px;
        height: 1200px;
        overflow: auto;
        box-sizing: border-box;
        padding: 24px;
        background: #111418;
      }
      svg {
        display: block;
      }
    </style>
  </head>
  <body>
    <main class="canvas">${svg.replace(/fill="white"/g, "fill=\"transparent\"")}</main>
  </body>
</html>`;
}

function defaultCfgDot() {
  return `digraph cfg {
  graph [rankdir=TB, bgcolor="transparent", pad="0.25", nodesep="0.85", ranksep="1.15", splines=true, outputorder=edgesfirst];
  node [shape=box, style="rounded,filled", color="#374151", fillcolor="#1d222a", fontcolor="#d6dce6", fontname="Menlo", fontsize=10, margin="0.10,0.08"];
  edge [color="#8aa0b8", fontcolor="#d6dce6", fontname="Menlo", fontsize=9, arrowsize=0.75, penwidth=1.5];

  entry [label="^entry\\l%0 = llvm.icmp eq %a, %b\\lllvm.cond_br %0, ^then, ^loop\\l"];
  then [label="^then\\l%1 = llvm.add %a, %b\\lllvm.br ^exit\\l"];
  loop [label="^loop\\l%2 = llvm.add %i, %one\\lllvm.cond_br %keep_going, ^loop, ^exit\\l"];
  exit [label="^exit\\lllvm.return\\l"];

  entry -> then [label="true"];
  entry -> loop [label="false"];
  then -> exit;
  loop:s -> loop:n [label="again"];
  loop -> exit [label="done"];
}`;
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (!arg.startsWith("--")) {
      continue;
    }
    const key = arg.slice(2);
    const value = rawArgs[index + 1];
    if (!value || value.startsWith("--")) {
      parsed[key] = "true";
    } else {
      parsed[key] = value;
      index += 1;
    }
  }
  return parsed;
}
