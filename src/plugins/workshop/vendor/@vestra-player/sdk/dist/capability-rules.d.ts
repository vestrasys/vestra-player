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
