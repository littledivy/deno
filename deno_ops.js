import { serve } from "https://deno.land/std/http/server.ts";
const { core } = Deno[Deno.internal]
const { ops } = core;

serve((req) => {
  const { socket, response } = Deno.upgradeWebSocket(req);

  socket.onopen = async () => {
    const symbols = Object.getOwnPropertySymbols(socket);
    const rid = socket[symbols[6]];

      ops.op_server_ws_next_event(
        rid,
        ([kind, value]) => {
            ops.op_server_ws_try_write_binary(rid, new Uint8Array(value));
        }      
      );
  };
  socket.onerror = (e) => {
    console.log(e);
  };

  return response;
}, { port: 8000 });
