import type { Needs } from "../connection.ts";
import type { ScopeImpl } from "../scope.ts";
import type { Disposable, EventPayload, FactEvent, HostEvent, SourceEvent } from "../types.ts";
/**
 * Every event a plugin listens to by name, and when its group is
 * subscribed at the player (`Needs`): `"held"` only while a listener, a
 * hold or a stream holds it (the SDK's own copy of a frequent state never
 * subscribes on its own); `null` from its first entry, a copy included.
 * A plugin listener subscribes its event either way.
 */
export declare const EVENT_NEEDS: Readonly<Record<HostEvent | SourceEvent | FactEvent, Needs>>;
/** `Player.on`'s arguments: a {@link HostEvent} name and a function. */
export declare function checkOn(event: unknown, listener: unknown): asserts event is HostEvent | SourceEvent | FactEvent;
/**
 * The discrete source events (player 0.10.0): their `state` reaches the
 * listener as a `PlayerState`, not in the wire shape.
 */
export declare const SOURCE_EVENTS: ReadonlySet<string>;
/**
 * The second batch of events (SDK 1.11.0): a fact that changed, once per
 * change. Some reach the listener in the SDK's shape (the scope converts
 * them, `ScopeImpl.fact`).
 */
export declare const FACT_EVENTS: ReadonlySet<string>;
/**
 * `Player.on`: `listener` hears each `event`, with a copy of its data;
 * `convert` (the source and fact events) turns the copy into what the
 * listener gets.
 */
export declare function playerOn<E extends HostEvent>(scope: ScopeImpl, event: E | SourceEvent | FactEvent, listener: (data: EventPayload<E>) => void, convert?: (data: unknown) => unknown): Disposable;
