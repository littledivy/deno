import fs from "node:fs";
import tls from "node:tls";

// connect to google.com
const socket = tls.connect(443, "google.com", {
  rejectUnauthorized: false,
});

// handle the connection
socket.on("connect", () => {
  console.log("Connected to google.com");
  try {
    socket.write(
      Buffer.from(
        "GET / HTTP/1.1\r\nHost: google.com\r\nConnection: close\r\n\r\n",
      ),
    );
  } catch (error) {
    console.error("Error writing to socket:", error);
  }

  console.log("Sent HTTP request to google.com");
});

socket.on("data", (data) => {
  console.log("Received data:");
  console.log(data.toString());
  socket.end();
});
