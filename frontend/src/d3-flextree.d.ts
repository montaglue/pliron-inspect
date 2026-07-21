declare module "d3-flextree" {
  export type FlextreeNode<T> = {
    data: T;
    x: number;
    y: number;
    depth: number;
    parent: FlextreeNode<T> | null;
    children?: FlextreeNode<T>[];
    descendants(): FlextreeNode<T>[];
  };

  export type FlextreeLink<T> = {
    source: FlextreeNode<T>;
    target: FlextreeNode<T>;
  };

  export type FlextreeHierarchy<T> = FlextreeNode<T> & {
    links(): FlextreeLink<T>[];
  };

  export type FlextreeLayout<T> = {
    (root: FlextreeHierarchy<T>): FlextreeHierarchy<T>;
    hierarchy(data: T): FlextreeHierarchy<T>;
  };

  export function flextree<T>(options: {
    children?: (node: T) => T[] | undefined;
    nodeSize?: (node: FlextreeNode<T>) => [number, number];
    spacing?: (nodeA: FlextreeNode<T>, nodeB: FlextreeNode<T>) => number;
  }): FlextreeLayout<T>;
}
