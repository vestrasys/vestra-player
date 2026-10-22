/**
 * Plugin versions and the version ranges of an index's `revoked` list, ported
 * unchanged from the plugins repository (`scripts/lib/semver.mjs`) so both
 * sides read a range the same way.
 *
 * A version is strict MAJOR.MINOR.PATCH without leading zeros. A range is one
 * or more alternatives separated by `||`; an alternative is `*` (every
 * version) or space-separated comparators that must all hold. A comparator is
 * a version with an optional `=` (default), `<`, `<=`, `>` or `>=`. No `^`,
 * `~`, `x` or hyphen ranges.
 */

const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const COMPARATOR = /^(<=|>=|<|>|=)?(.+)$/;

export type Comparator = { operator: string; version: string };

export function isVersion(value: unknown): value is string {
  return typeof value === "string" && VERSION.test(value);
}

function parts(version: string): number[] {
  const match = VERSION.exec(version);
  if (!match) throw new Error(`not a MAJOR.MINOR.PATCH version: ${version}`);
  return match.slice(1).map(Number);
}

/** Negative, zero or positive, like a sort comparator. */
export function compareVersions(a: string, b: string): number {
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i]! - y[i]!;
  return 0;
}

/** Alternatives of comparators; throws a readable message outside the grammar. */
export function parseRange(range: unknown): Comparator[][] {
  if (typeof range !== "string" || !range.trim()) throw new Error("range must be a non-empty string");
  if (range.length > 256) throw new Error("range is longer than 256 characters");
  return range.split("||").map((alternative) => {
    const tokens = alternative.trim().split(/\s+/).filter(Boolean);
    if (!tokens.length) throw new Error(`empty alternative in "${range}"`);
    if (tokens.length === 1 && tokens[0] === "*") return [];
    return tokens.map((token) => {
      const [, operator = "=", version] = COMPARATOR.exec(token)!;
      if (!isVersion(version))
        throw new Error(
          `"${token}" in "${range}" is not a comparator (an optional <, <=, >, >= or = and a MAJOR.MINOR.PATCH version)`,
        );
      return { operator, version };
    });
  });
}

/** Whether `version` is in `range`. */
export function satisfies(version: string, range: string): boolean {
  return parseRange(range).some((comparators) =>
    comparators.every(({ operator, version: bound }) => {
      const order = compareVersions(version, bound);
      switch (operator) {
        case "<":
          return order < 0;
        case "<=":
          return order <= 0;
        case ">":
          return order > 0;
        case ">=":
          return order >= 0;
        default:
          return order === 0;
      }
    }),
  );
}

/**
 * The player's own version against a minimum: only the three leading numbers
 * count (the host's rule; a pre-release suffix is ignored). An unreadable
 * player version is treated as new enough, an unreadable minimum as not met.
 */
export function playerAtLeast(player: string, minimum: string): boolean {
  const own = /^(\d+)\.(\d+)\.(\d+)/.exec(player);
  if (!own) return true;
  if (!isVersion(minimum)) return false;
  return compareVersions(own.slice(1).map(Number).join("."), minimum) >= 0;
}
