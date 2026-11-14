import type { ScopeImpl } from "../scope.ts";
import type { Disposable, EventPayload, HostEvent } from "../types.ts";
/**
 * Which events need event delivery (`"events"`: they reach the plugin only
 * after `plugin.player.subscribe()`, as they needed the `events`
 * declaration before 1.9.0); `null`: they arrive from the first listener.
 */
export declare const EVENT_NEEDS: Readonly<Record<HostEvent, "events" | null>>;
/** `Player.on`'s arguments: a {@link HostEvent} name and a function. */
export declare function checkOn(event: unknown, listener: unknown): asserts event is HostEvent;
/** `Player.on`: `listener` hears each `event`, with a copy of its data. */
export declare function playerOn<E extends HostEvent>(scope: ScopeImpl, event: E, listener: (data: EventPayload<E>) => void): Disposable;
