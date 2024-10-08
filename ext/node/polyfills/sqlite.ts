import {
  op_sqlite_exec,
  op_sqlite_get,
  op_sqlite_exec_noargs,
  op_sqlite_expandedsql,
  op_sqlite_open,
  op_sqlite_prepare,
} from "ext:core/ops";

export type SupportedValueType = null | number | string | bigint | Uint8Array;

interface DatabaseSyncOptions {
  open: boolean;
  enableForeignKeyConstraints: boolean;
}

export class DatabaseSync {
  #location: string;
  #enableForeignKeyConstraints: boolean;
  #handle: any;

  constructor(location: string, options?: DatabaseSyncOptions) {
    this.#location = location;
    this.#enableForeignKeyConstraints = options?.enableForeignKeyConstraints ??
      false;
    if (options?.open) {
      this.open();
    }
  }

  close() {}

  exec(sql: string, ...params: SupportedValueType[]) {
    if (params.length > 0) {
      op_sqlite_exec(this.#handle, sql, params);
    } else {
      op_sqlite_exec_noargs(this.#handle, sql);
    }
  }

  open() {
    this.#handle = op_sqlite_open(
      this.#location,
      this.#enableForeignKeyConstraints,
    );
  }

  prepare(sql: string) {
    const handle = op_sqlite_prepare(this.#handle, sql);
    return new StatementSync(handle);
  }
}

class StatementSync {
  #handle: any;

  constructor(handle: any) {
    this.#handle = handle;
  }

  all() {
  }
  expandedSQL(): string {
    return op_sqlite_expandedsql(this.#handle);
  }
  get(...params: SupportedValueType[]): unknown {
    return op_sqlite_get(this.#handle, params);
  }
  run() {
  }
  setAllowedBareNamedParameters(enabled: boolean) {
  }
  setReadBigInts(enabled: boolean) {
  }
  // TODO(@littledivy): Needs `sqlite_sql` bindings
  sourceSQL() {
    throw new Error("Not implemented");
  }
}

export default {
  DatabaseSync,
};
