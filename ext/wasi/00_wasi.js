// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

/// <reference path="../../core/internal.d.ts" />

"use strict";

((window) => {
  const core = window.Deno.core;
  const ops = core.ops;

  class Context {
    #rid;
    exports;
    #started;

    constructor(options = {}) {
      this.#started = false;
      const rid = ops.op_wasi_create(
        options.args ?? [],
        options.env ?? {},
        options.exitOnReturn ?? true,
        options.stdin ?? Deno.stdin.rid,
        options.stdout ?? Deno.stdout.rid,
        options.stderr ?? Deno.stderr.rid,
      );
      this.exports = {
        "args_get": (argvOffset, argvBufferOffset) => {
          return ops.op_wasi_args_get(rid, argvOffset, argvBufferOffset);
        },
        "args_sizes_get": (argcOffset, argvBufferSizeOffset) => {
          return ops.op_wasi_args_sizes_get(
            rid,
            argcOffset,
            argvBufferSizeOffset,
          );
        },
        "environ_get": (environOffset, environBufferOffset) => {
          return ops.op_wasi_environ_get(
            rid,
            environOffset,
            environBufferOffset,
          );
        },
        "environ_sizes_get": (environcOffset, environBufferSizeOffset) => {
          return ops.op_wasi_environ_sizes_get(
            rid,
            environcOffset,
            environBufferSizeOffset,
          );
        },
        "clock_res_get": (id, resolutionOffset) => {
          return ops.op_wasi_clock_res_get(rid, id, resolutionOffset);
        },
        "clock_time_get": (id, precision, timeOffset) => {
          return ops.op_wasi_clock_time_get(rid, id, precision, timeOffset);
        },
        "fd_advise": (_fd, _offset, _length, _advice) => {
          return ops.op_wasi_fd_advise(rid, _fd, _offset, _length, _advice);
        },
        "fd_allocate": (_fd, _offset, _length) => {
          return ops.op_wasi_fd_allocate(rid, _fd, _offset, _length);
        },
        "fd_close": (fd) => {
          return ops.op_wasi_fd_close(rid, fd);
        },
        "fd_datasync": (fd) => {
          return ops.op_wasi_fd_datasync(rid, fd);
        },
        "fd_fdstat_get": (fd, offset) => {
          return ops.op_wasi_fd_fdstat_get(rid, fd, offset);
        },
        "fd_fdstat_set_flags": (_fd, _flags) => {
          return ops.op_wasi_fd_fdstat_set_flags(rid, _fd, _flags);
        },
        "fd_fdstat_set_rights": (_fd, _rightsBase, _rightsInheriting) => {
          return ops.op_wasi_fd_fdstat_set_rights(
            rid,
            _fd,
            _rightsBase,
            _rightsInheriting,
          );
        },
        "fd_filestat_get": (fd, offset) => {
          return ops.op_wasi_fd_filestat_get(rid, fd, offset);
        },
        "fd_filestat_set_size": (fd, size) => {
          return ops.op_wasi_fd_filestat_set_size(rid, fd, size);
        },
        "fd_filestat_set_times": (fd, atim, mtim, flags) => {
          return ops.op_wasi_fd_filestat_set_times(rid, fd, atim, mtim, flags);
        },
        "fd_pread": (fd, iovsOffset, iovsLength, offset, nreadOffset) => {
          return ops.op_wasi_fd_pread(
            rid,
            fd,
            iovsOffset,
            iovsLength,
            offset,
            nreadOffset,
          );
        },
        "fd_prestat_get": (fd, prestatOffset) => {
          return ops.op_wasi_fd_prestat_get(rid, fd, prestatOffset);
        },
        "fd_prestat_dir_name": (fd, pathOffset, pathLength) => {
          return ops.op_wasi_fd_prestat_dir_name(
            rid,
            fd,
            pathOffset,
            pathLength,
          );
        },
        "fd_pwrite": (fd, iovsOffset, iovsLength, offset, nwrittenOffset) => {
          return ops.op_wasi_fd_pwrite(
            rid,
            fd,
            iovsOffset,
            iovsLength,
            offset,
            nwrittenOffset,
          );
        },
        "fd_read": (fd, iovsOffset, iovsLength, nreadOffset) => {
          return ops.op_wasi_fd_read(
            rid,
            fd,
            iovsOffset,
            iovsLength,
            nreadOffset,
          );
        },
        "fd_readdir": (
          fd,
          bufferOffset,
          bufferLength,
          cookie,
          bufferUsedOffset,
        ) => {
          return ops.op_wasi_fd_readdir(
            rid,
            fd,
            bufferOffset,
            bufferLength,
            cookie,
            bufferUsedOffset,
          );
        },
        "fd_renumber": (fd, to) => {
          return ops.op_wasi_fd_renumber(rid, fd, to);
        },
        "fd_seek": (fd, offset, whence, newOffsetOffset) => {
          return ops.op_wasi_fd_seek(rid, fd, offset, whence, newOffsetOffset);
        },
        "fd_sync": (fd) => {
          return ops.op_wasi_fd_sync(rid, fd);
        },
        "fd_tell": (fd, offsetOffset) => {
          return ops.op_wasi_fd_tell(rid, fd, offsetOffset);
        },
        "fd_write": (fd, iovsOffset, iovsLength, nwrittenOffset) => {
          return ops.op_wasi_fd_write(
            rid,
            fd,
            iovsOffset,
            iovsLength,
            nwrittenOffset,
          );
        },
        "path_create_directory": (fd, pathOffset, pathLength) => {
          return ops.op_wasi_path_create_directory(
            rid,
            fd,
            pathOffset,
            pathLength,
          );
        },
        "path_filestat_get": (
          fd,
          flags,
          pathOffset,
          pathLength,
          bufferOffset,
        ) => {
          return ops.op_wasi_path_filestat_get(
            rid,
            fd,
            flags,
            pathOffset,
            pathLength,
            bufferOffset,
          );
        },
        "path_filestat_set_times": (
          fd,
          flags,
          pathOffset,
          pathLength,
          atim,
          mtim,
          fstflags,
        ) => {
          return ops.op_wasi_path_filestat_set_times(
            rid,
            fd,
            flags,
            pathOffset,
            pathLength,
            atim,
            mtim,
            fstflags,
          );
        },
        "path_link": (
          oldFd,
          oldFlags,
          oldPathOffset,
          oldPathLength,
          newFd,
          newPathOffset,
          newPathLength,
        ) => {
          return ops.op_wasi_path_link(
            rid,
            oldFd,
            oldFlags,
            oldPathOffset,
            oldPathLength,
            newFd,
            newPathOffset,
            newPathLength,
          );
        },
        "path_open": (
          fd,
          dirflags,
          pathOffset,
          pathLength,
          oflags,
          rightsBase,
          rightsInheriting,
          fdflags,
          openedFdOffset,
        ) => {
          return ops.op_wasi_path_open(
            rid,
            fd,
            dirflags,
            pathOffset,
            pathLength,
            oflags,
            rightsBase,
            rightsInheriting,
            fdflags,
            openedFdOffset,
          );
        },
        "path_readlink": (
          fd,
          pathOffset,
          pathLength,
          bufferOffset,
          bufferLength,
          bufferUsedOffset,
        ) => {
          return ops.op_wasi_path_readlink(
            rid,
            fd,
            pathOffset,
            pathLength,
            bufferOffset,
            bufferLength,
            bufferUsedOffset,
          );
        },
        "path_remove_directory": (fd, pathOffset, pathLength) => {
          return ops.op_wasi_path_remove_directory(
            rid,
            fd,
            pathOffset,
            pathLength,
          );
        },
        "path_rename": (
          fd,
          oldPathOffset,
          oldPathLength,
          newFd,
          newPathOffset,
          newPathLength,
        ) => {
          return ops.op_wasi_path_rename(
            rid,
            fd,
            oldPathOffset,
            oldPathLength,
            newFd,
            newPathOffset,
            newPathLength,
          );
        },
        "path_symlink": (
          oldPathOffset,
          oldPathLength,
          fd,
          newPathOffset,
          newPathLength,
        ) => {
          return ops.op_wasi_path_symlink(
            rid,
            oldPathOffset,
            oldPathLength,
            fd,
            newPathOffset,
            newPathLength,
          );
        },
        "path_unlink_file": (fd, pathOffset, pathLength) => {
          return ops.op_wasi_path_unlink_file(rid, fd, pathOffset, pathLength);
        },
        "poll_oneoff": (
          _inOffset,
          _outOffset,
          _nsubscriptions,
          _neventsOffset,
        ) => {
          return ops.op_wasi_poll_oneoff(
            rid,
            _inOffset,
            _outOffset,
            _nsubscriptions,
            _neventsOffset,
          );
        },
        "proc_exit": (rval) => {
          return ops.op_wasi_proc_exit(rid, rval);
        },
        "proc_raise": (_sig) => {
          return ops.op_wasi_proc_raise(rid, _sig);
        },
        "sched_yield": () => {
          return ops.op_wasi_sched_yield(rid);
        },
        "random_get": (bufferOffset, bufferLength) => {
          return ops.op_wasi_random_get(rid, bufferOffset, bufferLength);
        },
        "sock_recv": (
          _fd,
          _riDataOffset,
          _riDataLength,
          _riFlags,
          _roDataLengthOffset,
          _roFlagsOffset,
        ) => {
          return ops.op_wasi_sock_recv(
            rid,
            _fd,
            _riDataOffset,
            _riDataLength,
            _riFlags,
            _roDataLengthOffset,
            _roFlagsOffset,
          );
        },
        "sock_send": (
          _fd,
          _siDataOffset,
          _siDataLength,
          _siFlags,
          _soDataLengthOffset,
        ) => {
          return ops.op_wasi_sock_send(
            rid,
            _fd,
            _siDataOffset,
            _siDataLength,
            _siFlags,
            _soDataLengthOffset,
          );
        },
        "sock_shutdown": (_fd, _how) => {
          return ops.op_wasi_sock_shutdown(rid, _fd, _how);
        },
      };
      this.#rid = rid;
    }

    start(instance) {
      if (this.#started) {
        throw new Error("WebAssembly.Instance has already started");
      }

      this.#started = true;

      const { _start, _initialize, memory } = instance.exports;

      if (!(memory instanceof WebAssembly.Memory)) {
        throw new TypeError("WebAsembly.instance must provide a memory export");
      }

      ops.op_wasi_set_memory(this.#rid, memory);

      if (typeof _initialize == "function") {
        throw new TypeError(
          "WebAsembly.instance export _initialize must not be a function",
        );
      }

      if (typeof _start != "function") {
        throw new TypeError(
          "WebAssembly.Instance export _start must be a function",
        );
      }

      _start();

      return null;
    }
  }
})(this);
