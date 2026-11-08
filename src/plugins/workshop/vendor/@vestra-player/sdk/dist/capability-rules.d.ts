import type { ApiRequest } from "./api-requests.ts";
/**
 * What a request needs, as the player's `capabilities.json` writes it:
 * `null` (nothing), a capability name, a choice by a string parameter, or a
 * choice by whether the plugin declared a capability.
 *
 * @internal
 */
export type Needs = null | string | {
    readonly param: string;
    readonly cases: Readonly<Record<string, string | null>>;
    readonly otherwise: string | null;
} | {
    readonly ifDeclared: string;
    readonly then: string;
    readonly otherwise: string;
};
/** @internal One RPC method of the player's table (`prefix.*` covers a prefix). */
export interface HostMethod {
    readonly name: string;
    readonly needs: Needs;
}
/** @internal One manifest capability of the player's table. */
export interface HostCapability {
    readonly name: string;
    readonly label: string;
    readonly risky: boolean;
    readonly requires: readonly string[];
    readonly residency: string;
    readonly methods: readonly HostMethod[];
}
/** @internal The player's `src/host/capabilities.json`. */
export interface HostCapabilities {
    readonly about: string;
    readonly apiVersion: string;
    readonly capabilities: readonly HostCapability[];
    readonly uncapable: readonly HostMethod[];
}
/** @internal Both tables; tests pass their own. */
export interface CapabilityRules {
    readonly host: HostCapabilities;
    readonly apis: Readonly<Record<string, readonly ApiRequest[]>>;
}
/** @internal The SDK's rules: the synced player table and `API_REQUESTS`. */
export declare const RULES: CapabilityRules;
/**
 * The player's entry for a method, as the player routes it: the exact name
 * first, then the first `prefix.*` entry whose prefix it starts with.
 * `undefined`: the player does not know the method.
 *
 * @internal
 */
export declare function hostMethod(method: string, host?: HostCapabilities): HostMethod | undefined;
/**
 * The capability one request needs, as the player decides it
 * (`capabilities::Route::needs`): `null` for none, `undefined` for a method
 * the player does not know.
 *
 * @param method - The wire method.
 * @param params - Its string parameters that a `Needs::Param` rule reads.
 * @param declared - Whether the plugin declared a capability.
 *
 * @internal
 */
export declare function requestNeeds(method: string, params: Readonly<Record<string, string>> | undefined, declared: (capability: string) => boolean, host?: HostCapabilities): string | null | undefined;
/**
 * One capability a member needs: the request that needs it, and the
 * capability whose `requires` brought it in (absent when the request needs
 * it directly).
 *
 * @internal
 */
export interface MemberNeed {
    readonly capability: string;
    readonly method: string;
    readonly requiredBy?: string;
}
/**
 * Every capability a member of the plugin API needs with the given manifest:
 * those its requests need and, through `requires`, those they need in turn.
 * `undefined`: the member is not in the table (it needs none).
 *
 * @param api - The member as `Interface.member` (`Player.seek`).
 * @param capabilities - The manifest's capabilities.
 *
 * @internal
 */
export declare function memberNeeds(api: string, capabilities: readonly string[], rules?: CapabilityRules): MemberNeed[] | undefined;
/**
 * The requests of a call to a member of `ARGUMENT_APIS` (SDK 1.7.0), given
 * the value of its first argument and the string fields of its second:
 * a subscribed event's `events.subscribe`, nothing for an event that
 * arrives unasked, or the named method. `undefined`: a method the player's
 * table does not know (not checked).
 *
 * @param kind - What the argument names (`ARGUMENT_APIS`).
 * @param name - The event or method name.
 * @param params - String fields of the second argument (`method` only).
 *
 * @internal
 */
export declare function argumentRequests(kind: "event" | "method", name: string, params?: Readonly<Record<string, string>>, rules?: CapabilityRules): ApiRequest[] | undefined;
/**
 * Every capability a list of requests needs with the given manifest (see
 * {@link memberNeeds}).
 *
 * @internal
 */
export declare function requestsNeed(requests: readonly ApiRequest[], capabilities: readonly string[], rules?: CapabilityRules): MemberNeed[];
/**
 * `CAPABILITY_INFO` from the tables: the label, highlight and requirements
 * from the player's table; the members from `API_REQUESTS` (every member a
 * request of which needs the capability, in the table's order; requests
 * marked `also` are left out, as they are documented at the member).
 *
 * @internal
 */
export declare function capabilityInfo(rules?: CapabilityRules): Record<string, {
    apis: string[];
    requires: string[];
    label: string;
    risky: boolean;
}>;
