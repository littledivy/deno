// Copyright 2018-2026 the Deno authors. MIT license.

import { primordials } from "ext:core/mod.js";
import {
  op_create_isolate,
  op_isolate_close,
  op_isolate_eval,
  op_isolate_exec_file,
  op_isolate_exec_npm,
} from "ext:core/ops";

const {
  ObjectFreeze,
  SymbolFor,
  TypeError,
} = primordials;

const _id = Symbol("[[isolateId]]");
const _closed = Symbol("[[closed]]");

class DenoIsolate {
  [_id];
  [_closed] = false;

  /** @internal */
  constructor(id) {
    this[_id] = id;
  }

  /**
   * Evaluate a JavaScript expression in the isolate and return the
   * JSON-serialized result.
   *
   * @param {string} code
   * @returns {Promise<any>}
   */
  async eval(code) {
    if (this[_closed]) {
      throw new TypeError("Isolate is closed");
    }
    if (typeof code !== "string") {
      throw new TypeError("Code must be a string");
    }
    const json = await op_isolate_eval(this[_id], code);
    if (json === "undefined") return undefined;
    return JSON.parse(json);
  }

  /**
   * Load and execute a file as the main ES module inside the isolate.
   *
   * @param {string} specifier - File path or URL to execute.
   * @returns {Promise<void>}
   */
  async execFile(specifier) {
    if (this[_closed]) {
      throw new TypeError("Isolate is closed");
    }
    if (typeof specifier !== "string") {
      throw new TypeError("Specifier must be a string");
    }
    await op_isolate_exec_file(this[_id], specifier);
  }

  /**
   * Resolve an npm package to its entry point and execute it inside the
   * isolate.  Accepts bare package names ("express"), scoped packages
   * ("@anthropic-ai/claude-code"), or explicit npm: specifiers
   * ("npm:chalk@5").
   *
   * @param {string} pkg - npm package specifier.
   * @returns {Promise<void>}
   */
  async execNpm(pkg) {
    if (this[_closed]) {
      throw new TypeError("Isolate is closed");
    }
    if (typeof pkg !== "string") {
      throw new TypeError("Package must be a string");
    }
    await op_isolate_exec_npm(this[_id], pkg);
  }

  /**
   * Terminate and clean up the isolate.
   */
  close() {
    if (this[_closed]) return;
    this[_closed] = true;
    op_isolate_close(this[_id]);
  }

  [SymbolFor("Deno.customInspect")]() {
    return `DenoIsolate { id: ${this[_id]} }`;
  }

  [Symbol.asyncDispose]() {
    this.close();
  }

  [Symbol.dispose]() {
    this.close();
  }
}

ObjectFreeze(DenoIsolate.prototype);

/**
 * Create a new lightweight V8 isolate with scoped permissions and
 * resource limits.
 *
 * @param {object} options
 * @param {object} [options.permissions] - Permission descriptors (read, write, net, etc.)
 * @param {object} [options.resources] - Resource limits (memoryLimitMb, cpuTimeoutMs)
 * @param {string[]} [options.builtins] - Allowed builtin modules
 * @param {boolean} [options.eval] - Allow eval()/new Function(). Default: false
 * @param {boolean} [options.nest] - Allow sub-isolate creation. Default: true
 * @returns {DenoIsolate}
 */
function run(options = {}) {
  const args = {
    permissions: options.permissions ?? null,
    resources: options.resources
      ? {
        memoryLimitMb: options.resources.memoryLimitMb ??
          options.resources.memoryLimit ?? null,
        cpuTimeoutMs: options.resources.cpuTimeoutMs ??
          options.resources.cpuTimeout ?? null,
      }
      : null,
    builtins: options.builtins ?? null,
    evalAllowed: options.eval ?? false,
    nest: options.nest ?? true,
  };

  const id = op_create_isolate(args);
  return new DenoIsolate(id);
}

export { DenoIsolate, run };
