import type { TestHost } from "@vestra-player/sdk/testing";

/** File system requests (realpath, stat, readFile…) libuv has not answered yet. */
export const filesPending = () => process.getActiveResourcesInfo().some((name) => name.startsWith("FSReq"));

/**
 * Make `host.settle()` also wait for the plugin's own file I/O. The playlist
 * resolves and stats paths before it adds rows or submits an open; the SDK's
 * settle() waits for host calls only, so on a loaded machine it could return
 * while a realpath was still on the thread pool. The test itself does no file
 * I/O while it settles, so any pending request is the plugin's.
 */
export function settleFiles(host: TestHost) {
  const settle = host.settle.bind(host);
  host.settle = async (ms = 0) => {
    await settle(ms);
    for (let quiet = 0; quiet < 2; ) {
      if (filesPending()) {
        quiet = 0;
        await new Promise((resolve) => setTimeout(resolve, 1));
      } else quiet++;
      await settle();
    }
  };
}
