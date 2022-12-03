# flash

Flash is a fast HTTP/1.1 server implementation for Deno.

```js
serve({ fetch: (req) => new Response("Hello World") });
```

# benchmarks on bare metal

## deno serve

```
taskset --cpu-list 14,15 wrk -c 256 -t 2 -d 10 http://127.0.0.1:4500/
Running 10s test @ http://127.0.0.1:4500/
  2 threads and 256 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   728.16us   31.78us   1.10ms   88.34%
    Req/Sec   176.40k     2.96k  213.27k    96.50%
  3511097 requests in 10.02s, 431.95MB read
Requests/sec: 350421.09
Transfer/sec:     43.11MB
```

## deno serve2

```
taskset --cpu-list 14,15 wrk -c 256 -t 2 -d 10 http://127.0.0.1:4500/
Running 10s test @ http://127.0.0.1:4500/
  2 threads and 256 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   684.79us   35.51us   0.99ms   90.77%
    Req/Sec   187.56k     3.73k  233.72k    96.50%
  3731773 requests in 10.02s, 459.10MB read
Requests/sec: 372296.11
Transfer/sec:     45.80MB
```

## bun

```
taskset --cpu-list 14,15 wrk -c 256 -t 2 -d 10 http://127.0.0.1:6000/
Running 10s test @ http://127.0.0.1:6000/
  2 threads and 256 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   673.02us  111.69us   6.24ms   98.01%
    Req/Sec   190.56k    13.24k  240.16k    97.50%
  3792545 requests in 10.03s, 466.57MB read
Requests/sec: 378198.34
Transfer/sec:     46.53MB
```
