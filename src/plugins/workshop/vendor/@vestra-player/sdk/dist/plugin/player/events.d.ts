import type { ScopeImpl } from "../scope.ts";
import type { Disposable, EventPayload, HostEvent, SourceEvent } from "../types.ts";
/**
 * Which events need event delivery (`"events"`: they reach the plugin only
 * after `plugin.player.subscribe()`, as they needed the `events`
 * declaration before 1.9.0); `null`: they arrive from the first listener.
 */
export declare const EVENT_NEEDS: Readonly<Record<HostEvent | SourceEvent, "events" | null>>;
/** `Player.on`'s arguments: a {@link HostEvent} name and a function. */
export declare function checkOn(event: unknown, listener: unknown): asserts event is HostEvent | SourceEvent;
/**
 * The discrete source events (player 0.10.0): their `state` reaches the
 * listener as a `PlayerState`, not in the wire shape.
 */
export declare const SOURCE_EVENTS: ReadonlySet<string>;
/**
 * `Player.on`: `listener` hears each `event`, with a copy of its data;
 * `convert` (the source events) turns the copy into what the listener
 * gets.
 */
export declare function playerOn<E extends HostEvent>(scope: ScopeImpl, event: E | SourceEvent, listener: (data: EventPayload<E>) => void, convert?: (data: unknown) => unknown): Disposable;
