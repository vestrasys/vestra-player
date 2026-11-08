import type { ScopeImpl } from "../scope.ts";
import type { Disposable, EventPayload, HostEvent } from "../types.ts";
/**
 * The capability each event needs, as the player checks `events.subscribe`
 * (`HOST_CAPABILITIES`: by event, otherwise `events`); `null`: none.
 */
export declare const EVENT_NEEDS: Readonly<Record<HostEvent, string | null>>;
/** `Player.on`: `listener` hears each `event`, with a copy of its data. */
export declare function playerOn<E extends HostEvent>(scope: ScopeImpl, event: E, listener: (data: EventPayload<E>) => void): Disposable;
