const tls = require('tls');
const fs = require('fs');

const ca = fs.readFileSync('ca.pem');

console.log('Connecting with servername: example.local.');
const socket = tls.connect({
  port: 8443,
  host: '127.0.0.1',
  servername: 'example.local.',
  ca: ca
}, () => {
  console.log('TLS connection established');
  console.log('Certificate authorized:', socket.authorized);
  
  socket.write('GET / HTTP/1.1\r\nHost: example.local\r\nConnection: close\r\n\r\n');
});

socket.on('secureConnect', () => {
  console.log('Connected to server');
  const cert = socket.getPeerCertificate();
	console.log(cert)
});

socket.on('data', (data) => {
  console.log('Received:', data.toString());
});

socket.on('end', () => {
  console.log('Connection ended');
});

socket.on('error', (err) => {
  console.error('Error:', err);
});
