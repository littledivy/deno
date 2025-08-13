// Copyright 2018-2025 the Deno authors. MIT license.

import {
  kStreamBaseField,
  LibuvStreamWrap,
} from "ext:deno_node/internal_binding/stream_wrap.ts";
import { providerType } from "ext:deno_node/internal_binding/async_wrap.ts";

export function wrap(
  handle,
  context,
  isServer,
  wrapHasActiveWriteFromPrevOwner,
) {
  return new TLSWrap(
    handle,
    context,
    isServer,
    wrapHasActiveWriteFromPrevOwner,
  );
}

export class TLSWrap extends LibuvStreamWrap {
  constructor(handle, context, isServer, wrapHasActiveWriteFromPrevOwner) {
    super(providerType.TLSWRAP, handle);

    this.handle = handle;
    this.context = context;
    this.isServer = isServer;
    this.wrapHasActiveWriteFromPrevOwner = wrapHasActiveWriteFromPrevOwner;
  }

  setVerifyMode() {}

  setNetPermToken() {}

  start() {
    const rid = this.handle._rid;
    const newRid = op_tls_start(rid);
  }
}

export default {
  wrap,
  TLSWrap,
};
