import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const mainPath = process.argv[2];
if (!mainPath) {
  process.stdout.write(JSON.stringify({ notes: ["missing plugin entry path"] }));
  process.exit(1);
}

const pluginDir = path.dirname(mainPath);
const ctxJson = process.env.XUANYU_HOOK_CTX || "{}";
const hookCtx = JSON.parse(ctxJson);
const fsAllowed = process.env.XUANYU_FS === "allow";
const networkAllowed = process.env.XUANYU_NETWORK !== "deny";
const processAllowed = process.env.XUANYU_PROCESS === "allow";

function blockedRequire(id) {
  if (id === "node:fs" || id === "fs") {
    if (!fsAllowed) {
      throw new Error("fs access denied in plugin sandbox");
    }
    return fs;
  }
  if (id === "node:path" || id === "path") {
    return path;
  }
  if (id.startsWith("node:")) {
    throw new Error(`module '${id}' blocked in plugin sandbox`);
  }
  throw new Error(`require('${id}') blocked in plugin sandbox`);
}

const sandbox = {
  module: { exports: {} },
  exports: {},
  require: blockedRequire,
  console: {
    log: (...args) => {
      process.stderr.write(`${args.map(String).join(" ")}\n`);
    },
  },
  process: {
    env: {
      XUANYU_HOOK: process.env.XUANYU_HOOK || "beforeTurn",
      XUANYU_HOOK_EVENT: process.env.XUANYU_HOOK_EVENT || "message.received",
      XUANYU_HOOK_CTX: ctxJson,
      XUANYU_PLUGIN_ID: process.env.XUANYU_PLUGIN_ID || "",
      XUANYU_PLUGIN_SANDBOX: "1",
      XUANYU_NETWORK: networkAllowed ? "allow" : "deny",
      XUANYU_FS: fsAllowed ? "allow" : "deny",
      XUANYU_PROCESS: processAllowed ? "allow" : "deny",
    },
  },
  __dirname: pluginDir,
  __filename: mainPath,
  Buffer,
  JSON,
  setTimeout,
  clearTimeout,
  fetch: networkAllowed
    ? (...args) => globalThis.fetch(...args)
    : () => {
        throw new Error("network access denied in plugin sandbox");
      },
};

const code = fs.readFileSync(mainPath, "utf8");
vm.runInNewContext(code, sandbox, {
  filename: mainPath,
  timeout: Number(process.env.XUANYU_PLUGIN_VM_TIMEOUT_MS || 7000),
});

const hook =
  sandbox.module?.exports?.beforeTurn ||
  sandbox.module?.exports?.onBeforeTurn ||
  sandbox.module?.exports?.default;

const notes = [];
if (typeof hook === "function") {
  const result = hook(hookCtx);
  if (result && Array.isArray(result.notes)) {
    notes.push(...result.notes.map(String));
  } else if (typeof result === "string" && result.trim()) {
    notes.push(result.trim());
  }
} else {
  notes.push("beforeTurn hook not exported");
}

process.stdout.write(JSON.stringify({ notes }));
