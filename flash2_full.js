// HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n
Deno.serve(() => new Response("Hello, World!"));
//Deno.serve2(() => new Response("HTTP/1.1 200 OK\r\nDate: Fri, 02 Dec 2022 22:17:19 GMT\r\nContent-Type: text/plain;charset=utf-8\r\nContent-Length: 13\r\n\r\nHello, World!"));
