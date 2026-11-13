/**
 * The most bytes (UTF-8) one answer of `docs` or `searchDocs` has.
 *
 * @category Docs
 */
export declare const DOCS_PAGE_BYTES = 8192;
/**
 * Which page of a long answer to return.
 *
 * @category Docs
 */
export interface DocsOptions {
    /** 1-based; the default is 1. */
    page?: number;
}
/**
 * The documentation at `path`: its text, then the paths one level down,
 * each with a one-line summary.
 *
 * `/` lists Overview, Guides and API. `/Guides/<group>/<page title>` is a
 * page of the documentation site (its file name works too:
 * `/Guides/Guides/testing`); `/API/<category>/<symbol>/<member>` is a
 * declaration with its signature, parameters, return value, errors,
 * player version and examples. Matching ignores case, spaces
 * and punctuation; a leading `/` and a `.md` suffix are optional; `.`
 * works like `/` (`Snapshot.export`); a name that is not at its level is
 * looked up below it (`TestHost`, `/API/Snapshot` finds
 * `/API/Media/Snapshot`), and one stale segment is skipped. A `#heading`
 * opens the page of a long answer that has it. A path that matches
 * nothing answers with the nearest names and the level's contents.
 *
 * @param path - Default `/`.
 * @param options - `page` of a long answer.
 * @returns At most {@link DOCS_PAGE_BYTES} bytes of text; a continued
 *   answer ends with the number of the next page.
 *
 * @category Docs
 */
export declare function docs(path?: string, options?: DocsOptions): string;
/**
 * Search the documentation by name and text: the matching paths, best
 * first, each with its one-line summary. Every word of `query` (common
 * words such as "a", "the", "to" left out) must occur, in any case, in
 * the path, the summary or the text; a match in a name ranks first, a
 * declaration's name before a page's.
 *
 * @param query - Words, such as `"snapshot export"` or `"onError"`.
 * @param options - `page` of a long answer.
 * @returns At most {@link DOCS_PAGE_BYTES} bytes of text.
 *
 * @category Docs
 */
export declare function searchDocs(query: string, options?: DocsOptions): string;
