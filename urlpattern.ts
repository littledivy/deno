import {URLPattern as URLPatternPolyfill} from "npm:urlpattern-polyfill";

Deno.bench("nop", function() {});

const pattern = new URLPattern({ pathname: "/" });
console.log(pattern.test({ pathname: "/" }))

Deno.bench("urlpattern - test", function() {
  pattern.test({ pathname: "/" })
});

Deno.bench("urlpattern - exec", function() {
  pattern.exec({ pathname: "/" })
});

const pattern2 = new URLPatternPolyfill({ pathname: "/" });
console.log(pattern2.test({ pathname: "/" }))

Deno.bench("urlpattern polyfill - test", function() {
  pattern2.test({ pathname: "/" })
});

Deno.bench("urlpattern polyfill - exec", function() {
  pattern2.exec({ pathname: "/" })
});
