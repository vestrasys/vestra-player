/**
 * The player's rules for its controls that the SDK checks before asking
 * (L1, refactor plan §2.2): shared by `plugin.player`, `plugin.host` and
 * `TestHost`.
 */
/** A step-seek length the player accepts: 0.1–3600 seconds in tenths. */
export declare const validStep: (seconds: unknown) => seconds is number;
