// The only file access the agent's tools have: everything stays inside the
// session's work folder. Paths are checked component by component (lstat):
// symbolic links and names starting with "." are refused, so the agent can
// neither leave the folder through a link nor create .pi/ or other hidden
// configuration. Node's permission model (--allow-fs-*) is the second belt.
import fs from "node:fs/promises";
import path from "node:path";

/**
 * Largest file the tools read or write. A plugin project has no use for a
 * bigger text file: the agent writes every file whole (write, edit), and edit
 * holds the old text, the new text and a diff in memory and in the session
 * file, so a larger file is runaway generated output, not source.
 */
export const MAX_FILE = 1024 * 1024;

/** A size for messages: "1.0 MB". */
export const megabytes = (bytes: number) => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

export class OutsideError extends Error {}

/** A file over its folder's size limit. */
export class TooLargeError extends Error {}

export class WorkFolder {
  readonly root: string;
  private constructor(root: string) {
    this.root = root;
  }

  /** `directory` must exist and not be a link; it is used by its real path. */
  static async open(directory: string): Promise<WorkFolder> {
    const stat = await fs.lstat(directory);
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new OutsideError("The working folder is not a plain folder");
    return new WorkFolder(await fs.realpath(directory));
  }

  /** Path relative to the work folder, for messages ("." for the folder). */
  relative(absolute: string): string {
    return path.relative(this.root, absolute).split(path.sep).join("/") || ".";
  }

  /**
   * The absolute path for `file` (absolute or relative to the folder), after
   * checking each existing component. `create`: missing components are fine.
   */
  async resolve(file: string, create = false): Promise<string> {
    const target = path.resolve(this.root, file);
    const rel = path.relative(this.root, target);
    if (rel === "") return this.checkRoot();
    if (rel.startsWith("..") || path.isAbsolute(rel)) throw new OutsideError(`Access denied: ${file} is outside the working folder`);
    const parts = rel.split(path.sep);
    let current = await this.checkRoot();
    for (const [index, part] of parts.entries()) {
      if (part.startsWith(".")) throw new OutsideError(`Access denied: names starting with "." are not allowed: ${part}`);
      current = path.join(current, part);
      let stat;
      try {
        stat = await fs.lstat(current);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        if (!create) throw Object.assign(new Error(`Not found: ${this.relative(current)}`), { code: "ENOENT" });
        return path.join(current, ...parts.slice(index + 1));
      }
      if (stat.isSymbolicLink()) throw new OutsideError(`Access denied: symbolic link: ${this.relative(current)}`);
      if (index < parts.length - 1 && !stat.isDirectory()) throw new OutsideError(`Not a folder: ${this.relative(current)}`);
    }
    return current;
  }

  private async checkRoot(): Promise<string> {
    if ((await fs.realpath(this.root)) !== this.root) throw new OutsideError("The working folder was replaced");
    return this.root;
  }

  /** The absolute path of `file` if it is a plain file within MAX_FILE, without reading it. */
  async readable(file: string): Promise<string> {
    const absolute = await this.resolve(file);
    const stat = await fs.stat(absolute);
    if (!stat.isFile()) throw new Error(`Not a file: ${this.relative(absolute)}`);
    if (stat.size > MAX_FILE) throw new TooLargeError(`File larger than ${megabytes(MAX_FILE)} (${megabytes(stat.size)}): ${this.relative(absolute)}`);
    return absolute;
  }

  async readFile(file: string): Promise<Buffer> {
    const absolute = await this.readable(file);
    const buffer = await fs.readFile(absolute);
    // It grew after the check.
    if (buffer.length > MAX_FILE) throw new TooLargeError(`File larger than ${megabytes(MAX_FILE)} (${megabytes(buffer.length)}): ${this.relative(absolute)}`);
    return buffer;
  }

  async writeFile(file: string, content: string): Promise<void> {
    if (Buffer.byteLength(content) > MAX_FILE) throw new TooLargeError(`Content larger than ${megabytes(MAX_FILE)}`);
    const absolute = await this.resolve(file, true);
    await this.mkdir(path.dirname(absolute));
    // Re-checked after mkdir: nothing on the way may have become a link.
    await this.resolve(absolute, true);
    await fs.writeFile(absolute, content);
  }

  /**
   * Deletes one file, or an empty folder. Not the work folder itself; a
   * link is refused by resolve() before anything is removed.
   */
  async remove(file: string): Promise<string> {
    const absolute = await this.resolve(file);
    if (absolute === this.root) throw new OutsideError("Cannot delete the working folder itself");
    const stat = await fs.lstat(absolute);
    if (stat.isDirectory()) {
      if ((await fs.readdir(absolute)).length) throw new Error(`Folder not empty: ${this.relative(absolute)} (delete its files first)`);
      await fs.rmdir(absolute);
    } else if (stat.isFile()) await fs.unlink(absolute);
    else throw new Error(`Not a plain file: ${this.relative(absolute)}`);
    return this.relative(absolute);
  }

  async mkdir(directory: string): Promise<void> {
    const absolute = await this.resolve(directory, true);
    const rel = path.relative(this.root, absolute);
    let current = this.root;
    for (const part of rel ? rel.split(path.sep) : []) {
      current = path.join(current, part);
      try {
        await fs.mkdir(current);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      }
      const stat = await fs.lstat(current);
      if (stat.isSymbolicLink() || !stat.isDirectory()) throw new OutsideError(`Not a folder: ${this.relative(current)}`);
    }
  }

  async exists(file: string): Promise<boolean> {
    try {
      await this.resolve(file);
      return true;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
      throw error;
    }
  }

  async isDirectory(file: string): Promise<boolean> {
    return (await fs.lstat(await this.resolve(file))).isDirectory();
  }

  /** Names in a folder, without hidden names and links. */
  async readdir(directory: string): Promise<string[]> {
    const absolute = await this.resolve(directory);
    const items = await fs.readdir(absolute, { withFileTypes: true });
    return items
      .filter((item) => !item.name.startsWith(".") && !item.isSymbolicLink())
      .map((item) => item.name)
      .sort();
  }

  /**
   * Every regular file under `directory` (relative to it, "/"-separated),
   * not following links, skipping hidden names and node_modules.
   */
  async walk(directory: string, limit: number): Promise<string[]> {
    const start = await this.resolve(directory);
    const out: string[] = [];
    const visit = async (dir: string) => {
      const items = (await fs.readdir(dir, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name));
      for (const item of items) {
        if (out.length >= limit) return;
        if (item.name.startsWith(".") || item.isSymbolicLink()) continue;
        const full = path.join(dir, item.name);
        if (item.isDirectory()) {
          if (item.name !== "node_modules") await visit(full);
        } else if (item.isFile()) out.push(path.relative(start, full).split(path.sep).join("/"));
      }
    };
    await visit(start);
    return out;
  }
}
