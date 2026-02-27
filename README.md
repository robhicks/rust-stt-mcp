# stt-mcp

A Model Context Protocol (MCP) server that records audio from your microphone and transcribes it to text using [Whisper](https://github.com/ggerganov/whisper.cpp). It communicates over stdio, making it easy to plug into Claude Code or any MCP-compatible client.

## Prerequisites

Fedora 43 with a working microphone. Install the build dependencies:

```bash
sudo dnf install alsa-lib-devel clang-devel cmake gcc-c++
```

You also need a Rust toolchain. If you don't have one:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Download the Whisper model

The server uses Whisper's `base` model by default. Download it to the expected location:

```bash
mkdir -p ~/.local/share/stt-mcp
curl -fSL -o ~/.local/share/stt-mcp/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

You can use a different model file by setting the `WHISPER_MODEL_PATH` environment variable.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/stt-mcp`.

## Configure Claude Code

Add the server to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "stt": {
      "command": "/home/YOU/dev/rust-stt-mcp/target/release/stt-mcp",
      "args": [],
      "env": {
        "WHISPER_MODEL_PATH": "/home/YOU/.local/share/stt-mcp/ggml-base.bin"
      }
    }
  }
}
```

Replace `/home/YOU` with your actual home directory.

## Test it

### Quick smoke test

Pipe an MCP `initialize` request into the server and confirm it responds:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' \
  | target/release/stt-mcp 2>/dev/null \
  | head -c 2000
```

You should see a JSON response containing `"serverInfo"` and `"tools"` in the capabilities.

### Test recording and transcription

Send an `initialize`, then call the `record_and_transcribe` tool. Speak into your microphone during the recording window:

```bash
{
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}'
  sleep 0.5
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"record_and_transcribe","arguments":{"duration_secs":3,"language":"en"}}}'
  sleep 5
} | target/release/stt-mcp 2>/dev/null
```

The second JSON response will contain the transcribed text from your microphone.

### Test from Claude Code

Once configured in `settings.json`, restart Claude Code and ask it to use the `record_and_transcribe` tool. For example:

> "Use the stt server to record 5 seconds of audio and transcribe it."

## Tool reference

### `record_and_transcribe`

Records audio from the default microphone and returns transcribed text.

| Parameter       | Type   | Default | Description                                      |
|-----------------|--------|---------|--------------------------------------------------|
| `duration_secs` | number | `5`     | How many seconds to record                       |
| `language`      | string | `"en"`  | Language hint for Whisper (e.g. `"en"`, `"es"`)  |
