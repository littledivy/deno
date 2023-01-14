## flash_dev

Standalone HTTP server runtime. 

* Release builds in <5s.
* Inbuilt tools for profiling and generating graphs.

Usage:

```rust
const { Response } = this.__bootstrap.fetch;
const { createServe } = this.__bootstrap.flash;
const serve = createServe();

serve(function () {
  return new Response("Hello World");
});
```

Building:

```
./build.ts
./build.ts release
```

Note: The build script checks for a proper Deno checkout.
