export function add(a: number, b: number): number {
  return a + b;
}

export const total: number = [1, 2, 3].reduce(add, 0);
