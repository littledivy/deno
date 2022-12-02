const raw = new TextEncoder().encode("HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHello World");
Deno.serve(() => new Response(raw));
