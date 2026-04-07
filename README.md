# `protobug`

> Protobuf Debugging Suite

[![asciicast](https://asciinema.org/a/2Xesc9SvbYwvIDri.svg)](https://asciinema.org/a/2Xesc9SvbYwvIDri)

`protobug` is a schema-aware CLI for inspecting and rewriting protobuf payloads. It can decode binary, hex, and base64 payloads, project them into canonical JSON, apply `jaq` filters to that JSON, and re-encode the result back into protobuf bytes.

## Current Feature Set

- Inspect protobuf payloads with a schema-aware TUI.
- Print protobuf payloads as canonical JSON, raw binary, hex, or base64.
- Edit messages by applying `jaq` filters to their JSON representation.
- Rewrite files in place while preserving their original encoding.
- Work with line-delimited hex/base64 files as multiple independent messages.
- Navigate multiple messages in the inspector one at a time.

## Commands

### `inspect`

`inspect` is the interactive path. It loads a protobuf payload with a schema and either opens the TUI or prints the decoded message in another format.

Supported input formats:

- `auto`
- `binary`
- `hex`
- `base64`

Supported print formats:

- `json`
- `binary`
- `hex`
- `base64`

### `edit`

`edit` is the non-interactive transformation path. It loads a message, converts it to JSON, optionally runs a `jaq` filter, and emits the result in the requested format or writes it back in place.

Supported input formats:

- `auto`
- `json`
- `binary`
- `hex`
- `base64`

Supported output formats:

- `json`
- `binary`
- `hex`
- `base64`

## Examples

Print a protobuf payload as canonical JSON:

```bash
protobug inspect \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file event.bin \
  --input-format binary \
  --print-format json
```

Open the interactive inspector:

```bash
protobug inspect \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file event.hex \
  --input-format hex
```

Inspect a line-delimited base64 file as multiple messages:

```bash
protobug inspect \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file events.b64 \
  --input-format base64 \
  --multiple
```

Convert JSON back into protobuf bytes:

```bash
protobug edit \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file event.json \
  --input-format json \
  --print-format binary > event.bin
```

Apply a `jaq` filter and print the edited message as JSON:

```bash
protobug edit \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file event.bin \
  --input-format binary \
  --filter '.click |= (.x as $x | .y as $y | .x = $y | .y = $x)' \
  --print-format json
```

Rewrite a protobuf file in place:

```bash
protobug edit \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file event.hex \
  --input-format hex \
  --filter '.reason = "updated"' \
  --in-place
```

Rewrite each message in a line-delimited base64 file independently:

```bash
protobug edit \
  --schema protogen/proto/system-event.proto \
  --message SystemEvent \
  --file events.b64 \
  --input-format base64 \
  --multiple \
  --filter '.click.x += 10' \
  --in-place
```

Use the bundled `just` helpers during development:

```bash
just proto-json protogen/proto/system-event.proto event.bin SystemEvent binary
just proto-jq protogen/proto/system-event.proto event.bin '.click.x += 10' SystemEvent binary
just proto-jq-rewrite protogen/proto/system-event.proto event.hex '.reason = "updated"' SystemEvent hex
```

## TUI Notes

In the inspector:

- `Ctrl-S` saves configured outputs.
- `Ctrl-X` toggles the hex pane.
- `Ctrl-A` toggles the ASCII pane.
- `[` and `]` adjust bytes-per-row.
- `Ctrl-J` and `Ctrl-K` move between messages in multi-message mode.
- `Ctrl-G` opens the message picker.

In the message picker:

- `Enter` jumps to a specific message number.
- `Ctrl-B` jumps to the first message.
- `Ctrl-E` jumps to the last message.
- `Esc` cancels.
