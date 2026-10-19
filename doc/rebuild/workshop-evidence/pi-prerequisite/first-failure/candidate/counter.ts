// Standalone counter prerequisite, not an installable plugin.
const step: number = "1";
let count: number = 0;
export function increment(): number { count += step; return count; }
export function reset(): number { count = 0; return count; }
