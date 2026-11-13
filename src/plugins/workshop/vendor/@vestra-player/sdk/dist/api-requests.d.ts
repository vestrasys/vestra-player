/**
 * Which player requests each member of the plugin API sends, written
 * `Interface.member`; members declared on `Scope` are written `Plugin.…`.
 * The generated reference pages read it: the Stability column of the
 * message table (docs/reference/messages.md) and the members of each event
 * (docs/reference/events.md). For a listener the row is the
 * `events.subscribe` with its event name.
 *
 * A row lists the request the member sends to do its work. Members whose
 * requests come from a handle another member returned (a `Window` from
 * `Windows.create`) are listed in the message table's `WRAPPED_BY`
 * (scripts/doc-tables/messages.mjs) instead.
 *
 * @internal
 */
/** @internal One request a member sends. */
export interface ApiRequest {
    /** The wire method (`source.submit`). */
    readonly method: string;
    /** String parameters that name what is meant (`event`, `stage`). */
    readonly params?: Readonly<Record<string, string>>;
}
/** @internal */
export declare const API_REQUESTS: Readonly<Record<string, readonly ApiRequest[]>>;
/**
 * The events whose listener subscribes (`events.subscribe` with the event
 * name): the plugin API's `HostEvent`. Other names a plugin listens to
 * (`commands.execute`, `ui.callback`) arrive unasked and need nothing.
 * test/wire-facade.test.ts keeps it equal to `HostEvent`.
 *
 * @internal
 */
export declare const SUBSCRIBED_EVENTS: readonly string[];
