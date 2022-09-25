Deno.serve((r) => {
  const { response, socket } = Deno.upgradeWebSocket(r);
  socket.onmessage = (e) => {
//    console.log(e.data)
    socket.send(e.data);
  };
  socket.onopen = () => {
    socket.send("open")
    console.log("open")
  }
  return response;
}, { port: 8000 });
