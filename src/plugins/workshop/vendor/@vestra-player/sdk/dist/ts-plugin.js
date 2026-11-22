/** @internal The plugin factory tsserver calls (`ts-plugin/index.js`). */
export default function init(_modules) {
    return {
        create(info) {
            info.project?.projectService?.logger?.info("[@vestra-player/sdk/ts-plugin] does nothing since SDK 1.9.0; remove it from tsconfig.json");
            return info.languageService;
        },
    };
}
