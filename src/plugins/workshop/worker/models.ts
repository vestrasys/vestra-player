// The model object pi runs: from its catalog, or, for a model the provider
// listed that the catalog does not know yet (pi's catalog lags behind), a
// copy of one of the provider's catalog models (endpoint, API, limits) with
// the listed id.
import type { ModelRuntime } from "../vendor/pi/pi.mjs";

type Model = NonNullable<ReturnType<ModelRuntime["getModel"]>>;

export function resolveModel(runtime: ModelRuntime, provider: string, id: string): Model | undefined {
  const known = runtime.getModel(provider, id);
  if (known) return known;
  const models = runtime.getModels(provider);
  if (!models.length) return undefined;
  // A template of the API most of the provider's models use (where its list came from).
  const count = new Map<string, number>();
  for (const m of models) count.set(m.api, (count.get(m.api) ?? 0) + 1);
  const api = [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0];
  const template = models.find((m) => m.api === api)!;
  return { ...template, id, name: id };
}
