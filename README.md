# XRTC

It enables you to transmit TCP packages over WebRTC datachannel.

## Demo

```shell
# Run a simple http server on 8000 port
python -m http.server 8000

# Run a server node in another terminal
cargo run -- server

# Run a client node in another terminal
cargo run -- client --target 127.0.0.1:8000 --listen 127.0.0.1:5555

# Again, in another terminal, access the http server via proxy
# [client] -> [server] -> [http server]
curl http://127.0.0.1:5555
```
