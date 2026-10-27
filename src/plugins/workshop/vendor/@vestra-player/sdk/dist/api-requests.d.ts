/**
 * Which player requests each member of the plugin API sends: the SDK's half
 * of the capability rule (the player's half, what each request needs, is
 * `HOST_CAPABILITIES`). A member is written `Interface.member` as in
 * `CAPABILITY_INFO`; members declared on `Scope` are written `Plugin.…`.
 *
 * List the requests that decide the member's capabilities: the one it
 * sends to do its work, and for a listener the `events.subscribe` with its
 * event name. A string parameter that the player's rule reads (`event`,
 * `stage`) is given in `params`. `also: true` marks a request whose
 * capability the member needs on top of its own (documented at the member,
 * not listed in its capability's `CAPABILITY_INFO.apis`).
 *
 * Members that need no capability are left out, apart from a few that show
 * a rule's "none" case. Members of objects that another listed member
 * returns (a `Window` from `Windows.create`, a `Panel` from `Panels.mount`)
 * are left out too: the call that created the object already needs the
 * capability.
 *
 * Order matters for `CAPABILITY_INFO`: each capability lists its members in
 * this table's order. test/capability-rules.test.ts checks every method and
 * parameter against the player's table.
 *
 * @internal
 */
/** @internal One request a member sends. */
export interface ApiRequest {
    /** The wire method (`source.submit`). */
    readonly method: string;
    /** String parameters the player's rule for the method reads. */
    readonly params?: Readonly<Record<string, string>>;
    /** A further capability, documented at the member. */
    readonly also?: true;
}
/** @internal */
export declare const API_REQUESTS: Readonly<Record<string, readonly ApiRequest[]>>;
