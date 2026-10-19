import { realpath } from "node:fs/promises";
import { basename, resolve } from "node:path";
export const sameStamp = (a, b) => a.generation === b.generation && a.intentVersion === b.intentVersion;
/** All ordering, selection and source association belong to this instance. */
export class Queue {
    rows = [];
    selected = null;
    binding = null;
    failed = null;
    revision = 0;
    epoch = 0;
    message = "添加文件或链接；选择后点击播放";
    nextId = 0;
    row(id) {
        return this.rows.find((row) => row.rowId === id);
    }
    select(id) {
        if (this.row(id))
            this.selected = id;
    }
    neighbor(delta) {
        const anchor = this.failed ?? this.binding?.rowId ?? this.selected;
        const index = this.rows.findIndex((row) => row.rowId === anchor);
        return index < 0 ? undefined : this.rows[index + delta];
    }
    selectNeighbor(delta) {
        const index = this.rows.findIndex((row) => row.rowId === this.selected);
        const next = this.rows[index < 0
            ? 0
            : Math.max(0, Math.min(this.rows.length - 1, index + delta))];
        if (next)
            this.selected = next.rowId;
    }
    async addFiles(paths, stillActive = () => true) {
        let added = 0, duplicate = 0, invalid = 0;
        const wasEmpty = this.rows.length === 0;
        let first = null;
        for (const path of paths) {
            const source = resolve(path);
            let key = source, error = "";
            try {
                key = await realpath(source);
            }
            catch {
                error = "文件不存在或暂不可读取，可重试";
            }
            if (!stillActive())
                return;
            if (this.rows.some((row) => row.kind === "file" && row.key === key)) {
                duplicate++;
                continue;
            }
            const row = this.append({
                source,
                key,
                title: basename(source),
                kind: "file",
                error,
            });
            first ??= row.rowId;
            added++;
            if (error)
                invalid++;
        }
        if (wasEmpty && first)
            this.selected = first;
        this.message = `新增 ${added} 项，跳过 ${duplicate} 个重复，${invalid} 项待重试`;
    }
    addUrl(input) {
        const source = input.trim();
        let url;
        try {
            url = new URL(source);
        }
        catch {
            throw new Error("请输入完整 HTTP(S) 链接");
        }
        if (!["http:", "https:"].includes(url.protocol) ||
            url.username ||
            url.password)
            throw new Error("只接受不含账户密码的 HTTP(S) 链接");
        const key = url.href;
        const existing = this.rows.find((row) => row.kind === "url" && row.key === key);
        const row = existing ??
            this.append({ source, key, title: source, kind: "url", error: "" });
        this.selected = row.rowId;
        this.message = existing ? "已选择已有链接" : "链接已添加，点击播放以打开";
    }
    append(input) {
        const row = {
            ...input,
            rowId: `row-${++this.nextId}`,
            status: input.error ? "待重试" : "待播放",
        };
        this.rows.push(row);
        this.revision++;
        return row;
    }
    move(delta) {
        const index = this.rows.findIndex((row) => row.rowId === this.selected);
        const other = index + delta;
        if (index < 0 || other < 0 || other >= this.rows.length)
            return;
        [this.rows[index], this.rows[other]] = [this.rows[other], this.rows[index]];
        this.revision++;
    }
    remove() {
        const index = this.rows.findIndex((row) => row.rowId === this.selected);
        if (index < 0)
            return;
        const [removed] = this.rows.splice(index, 1);
        if (removed.rowId === this.binding?.rowId)
            this.detach("当前媒体已移出列表");
        if (removed.rowId === this.failed)
            this.failed = null;
        this.selected =
            this.rows[Math.min(index, this.rows.length - 1)]?.rowId ?? null;
        this.revision++;
    }
    clear() {
        this.rows = [];
        this.selected = null;
        this.failed = null;
        this.detach("列表已清空，当前媒体继续");
        this.revision++;
    }
    detach(message) {
        const row = this.row(this.binding?.rowId ?? null);
        if (row)
            row.status = "已切换";
        this.binding = null;
        this.epoch++;
        this.message = message;
    }
    begin(rowId) {
        this.detach("正在打开…");
        this.failed = null;
        const row = this.row(rowId);
        if (row) {
            row.status = "正在打开";
            row.error = "";
        }
        return this.epoch;
    }
    fail(rowId, message) {
        const row = this.row(rowId);
        if (!row)
            return;
        row.status = "失败";
        row.error = message;
        this.failed = rowId;
        this.binding = null;
        this.message = message;
    }
    /** Return a single natural-end token. Caller rechecks it before conditional open. */
    observe(state) {
        const binding = this.binding;
        if (!binding)
            return null;
        const row = this.row(binding.rowId);
        if (!row) {
            this.detach("当前媒体已移出列表");
            return null;
        }
        if (state.stamp.generation !== binding.ticket.stamp.generation ||
            state.kind !== "media") {
            this.failed = null;
            this.detach("已切换到列表外来源");
            return null;
        }
        // Public restart keeps generation but advances cycle/intent. The new cycle
        // gets fresh end eligibility; old deferred progression fails its stamp check.
        if (!sameStamp(state.stamp, binding.ticket.stamp) ||
            state.cycle !== binding.ticket.cycle) {
            this.epoch++;
            binding.ticket = {
                ...binding.ticket,
                stamp: state.stamp,
                cycle: state.cycle,
            };
            binding.endedCycle = null;
        }
        if (state.phase === "error") {
            this.fail(row.rowId, typeof state.terminalReason === "object" && state.terminalReason
                ? state.terminalReason.failed
                : "无法打开此媒体");
        }
        else if (state.phase === "ready") {
            row.status = state.paused ? "已暂停" : "播放中";
            row.error = "";
            this.message = state.paused ? "已暂停" : "播放中";
        }
        else if (state.phase === "loading") {
            row.status = "正在打开";
        }
        else if (state.phase === "ended") {
            row.status = "已结束";
            if (state.terminalReason === "natural" &&
                binding.endedCycle !== state.cycle) {
                binding.endedCycle = state.cycle;
                return {
                    rowId: row.rowId,
                    stamp: { ...state.stamp },
                    cycle: state.cycle,
                    epoch: this.epoch,
                };
            }
        }
        else if (state.phase === "empty") {
            this.detach("当前来源已关闭");
        }
        return null;
    }
}
