// The only file access the agent's tools have: everything stays inside the
// session's work folder. Paths are checked component by component (lstat):
// symbolic links and names starting with "." are refused, so the agent can
// neither leave the folder through a link nor create .pi/ or other hidden
// configuration. Node's permission model (--allow-fs-*) is the second belt.
import fs from "node:fs/promises";
import path from "node:path";

/** Largest file the tools read or write. */
export const MAX_FILE = 1024 * 1024;

export class OutsideError extends Error {}

export class WorkFolder {
  readonly root: string;
  private constructor(root: string) {
    this.root = root;
  }

  /** `directory` must exist and not be a link; it is used by its real path. */
  static async open(directory: string): Promise<WorkFolder> {
    const stat = await fs.lstat(directory);
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new OutsideError("工作目录不是普通文件夹");
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
    if (rel.startsWith("..") || path.isAbsolute(rel)) throw new OutsideError(`拒绝访问工作目录以外的路径：${file}`);
    const parts = rel.split(path.sep);
    let current = await this.checkRoot();
    for (const [index, part] of parts.entries()) {
      if (part.startsWith(".")) throw new OutsideError(`拒绝以「.」开头的名称：${part}`);
      current = path.join(current, part);
      let stat;
      try {
        stat = await fs.lstat(current);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        if (!create) throw Object.assign(new Error(`不存在：${this.relative(current)}`), { code: "ENOENT" });
        return path.join(current, ...parts.slice(index + 1));
      }
      if (stat.isSymbolicLink()) throw new OutsideError(`拒绝符号链接：${this.relative(current)}`);
      if (index < parts.length - 1 && !stat.isDirectory()) throw new OutsideError(`不是文件夹：${this.relative(current)}`);
    }
    return current;
  }

  private async checkRoot(): Promise<string> {
    if ((await fs.realpath(this.root)) !== this.root) throw new OutsideError("工作目录已被替换");
    return this.root;
  }

  async readFile(file: string): Promise<Buffer> {
    const absolute = await this.resolve(file);
    const stat = await fs.stat(absolute);
    if (!stat.isFile()) throw new Error(`不是文件：${this.relative(absolute)}`);
    if (stat.size > MAX_FILE) throw new Error(`文件超过 1 MB：${this.relative(absolute)}`);
    return fs.readFile(absolute);
  }

  async writeFile(file: string, content: string): Promise<void> {
    if (Buffer.byteLength(content) > MAX_FILE) throw new Error("写入内容超过 1 MB");
    const absolute = await this.resolve(file, true);
    await this.mkdir(path.dirname(absolute));
    // Re-checked after mkdir: nothing on the way may have become a link.
    await this.resolve(absolute, true);
    await fs.writeFile(absolute, content);
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
      if (stat.isSymbolicLink() || !stat.isDirectory()) throw new OutsideError(`不是文件夹：${this.relative(current)}`);
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
