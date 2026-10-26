// Preloaded (--import) into the plugin CLI the toolchain runs from vendor/.
// vendor/ is laid out like node_modules without being one, and the CLI finds
// its packages through NODE_PATH. NODE_PATH only serves require(); an ES
// module import of a bare name (the SDK's dist/i18n-tools.js has
// `import ts from "typescript"`) walks up the node_modules folders above the
// importing file and does not look there. A development checkout has one
// (src/node_modules) and passes; an installed player has none, and the CLI
// died before printing anything (Windows, SDK 1.5.17).
//
// When Node's own resolution does not find a bare name, it is resolved the
// way require() would, which reads NODE_PATH. Everything Node finds itself
// stays as it is.
import { createRequire, registerHooks } from "node:module";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
/** Relative, absolute (POSIX or a drive letter), URL-like and #imports specifiers are not bare names. */
const NOT_BARE = /^(\.|\/|\\|#|[A-Za-z][A-Za-z0-9+.-]*:)/;

registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ERR_MODULE_NOT_FOUND" || NOT_BARE.test(specifier)) throw error;
      let file: string;
      try {
        file = require.resolve(specifier);
      } catch {
        throw error;
      }
      return { url: pathToFileURL(file).href, shortCircuit: true };
    }
  },
});
