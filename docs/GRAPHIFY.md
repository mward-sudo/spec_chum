# Graphify workflow

Install Graphify with its optional OpenAI-compatible backend so OpenRouter
full refreshes can run:

```sh
uv tool install --force "graphifyy[openai]"
```

Then ensure the tool's bin directory is on `PATH`. Check with
`command -v graphify` and `graphify --version`.

Use the repository wrapper to refresh the checked-in graph:

```sh
./scripts/graphify_update.sh          # incremental code/AST update
./scripts/graphify_update.sh --full   # optional full extraction; may call an LLM
```

The default provider behavior is unchanged. For a full refresh through
OpenRouter, graphifyy 0.9.69 can use its OpenAI-compatible endpoint. Set the
key in the shell environment without putting its value in a command line, then
opt in:

```sh
printf 'OpenRouter API key: ' >&2
read -rs OPENROUTER_API_KEY
echo
export OPENROUTER_API_KEY
export GRAPHIFY_PROVIDER=openrouter
export GRAPHIFY_OPENROUTER_MODEL='provider/model-id'
./scripts/graphify_update.sh --full
unset OPENROUTER_API_KEY GRAPHIFY_PROVIDER GRAPHIFY_OPENROUTER_MODEL
```

`GRAPHIFY_OPENROUTER_MODEL` is optional. If omitted, an existing
`GRAPHIFY_OPENAI_MODEL` or `OPENAI_MODEL` is used; otherwise the wrapper uses
OpenRouter's `openai/gpt-4.1-mini` model slug. The wrapper maps the key to
`OPENAI_API_KEY` and sets `OPENAI_BASE_URL=https://openrouter.ai/api/v1` only
for the opt-in provider.
It assigns the secret using a shell builtin and invokes Graphify without the
secret in its arguments. Avoid shell tracing (`set -x`) while the key is in
the environment. Full extraction can incur API charges; AST-only updates do
not need an API key. Run a full refresh after documentation changes only when
those sources are represented in the graph; ordinary prose edits do not need
an LLM-backed refresh.

The default workspace gate runs `scripts/test_graphify_update.sh`, which uses a
temporary fake `graphify` executable to check routing, model precedence,
missing-key behavior, and that the key does not appear in arguments or output.
It makes no network request and needs no API key.

Graph outputs are committed under `graphify-out/`. See `AGENTS.md` for the
project's graph artifact and review conventions.
