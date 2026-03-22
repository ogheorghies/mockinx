# mockinx - compliant web server, broken on purpose

\[MO-keen-x\]: (1) set reply rules (2) get back exactly that.

all good * CRUD * slowness * drops * throttling * trickling * chaos

```bash
cargo install mockinx yurl    # yurl instead of curl
mockinx 9999                  # start server on port 9999
```

```bash
echo '{p: localhost:9999/_mx, b: {
  match: {g: /toys/3},
  reply: {s: 200, b: {name: Owl, price: 5.99}}
}}' | yurl # (1) reply rule for /toys/3

echo '{g: localhost:9999/toys/3}' | yurl # (2) hit /toys/3
# → {"name": "Owl", "price": 5.99}
```

## Reply rule

A reply rule encompasses four aspects:

```yaml
match: {}   # what to match     (method, URI)
reply: {}   # what to respond   (status, payload)
serve: {}   # how to serve it   (speed, chunks, ...)
chaos: []   # what can go wrong (probability → override)
```

### match

```yaml
match: {g: /api/data}        # GET /api/data
match: {p: /api/data}        # POST /api/data
match: {_: /api/data}        # any method, /api/data
match: _                     # match everything
```

### reply

Reply is polymorphic — object, array, or crud.

`yttp` conventions are used: `s:` `h:` `b:`, shortcuts. 
`mockinx` directives also use `!` suffix to distinguish from literal data.

```yaml
# static reply (content-type inferred from body; explicit h: overrides)
reply: {s: 200, h: {ct!: j!}, b: {"items": [1, 2, 3]}}

# status only
reply: {s: 204}

# generated body (rand!, pattern! are mockinx directives)
reply: {s: 200, b: {rand!: {size: 10kb, seed: 7}}}
reply: {s: 200, b: {pattern!: {repeat: "abc", size: 1mb}}}

# wrong content-type (malformed response)
reply: {s: 200, h: {ct!: h!}, b: '{"valid": "json"}'}

# sequence — array of replies, cycled per connection
reply:
  - {s: 401, b: "unauthorized"}
  - {s: 200, b: "ok"}

# crud — in-memory REST resource
reply: {crud!: {seed: [{id: 1, name: Ball}, {id: 3, name: Owl}]}}
reply: {crud!: {id: {name: sku, new: auto}}}
```

### serve

How the response is served — delivery shaping and operational constraints:

```yaml
serve:
  # delivery shaping
  span: 5s                              # pacing: duration target (auto-chunked)
  span: {chunk: 1kb, delay: 100ms}      # pacing: explicit chunking
  span: {speed: 10kb/s}                 # pacing: bandwidth cap
  drop: 2kb                             # kill connection after N bytes
  drop: 1s                              # kill connection after N time
  first_byte: 2s                        # delay before first byte

  # operational constraints (connections, rate per second)
  conn: {max: 5, over: {s: 429, b: "too many"}}
  conn: {max: 5, over: block}
  conn: {max: 5, over: {block: 3s, then: {s: 429, b: "timeout"}}}
  rps: {max: 100, over: {s: 429}}
  timeout: 30s
```

Any scalar value supports jitter via ranges (`min..max` or `value..percent`):

```yaml
serve:
  span: 4s..6s                                  # random timespan
  span: {chunk: 512b..2kb, delay: 50ms..150ms}  # random chunk size and delay
  span: {speed: 10kb/s..20%}                    # random bandwidth 8kb/s..12kb/s
  drop: 1kb..4kb                                # drop conn anywhere in that byte range
  first_byte: 1s..10%                            # 900ms..1.1s
```

### chaos

Probabilistic overrides for reply and/or serve. Each entry has a weight (`p`)
and optional `reply`/`serve` overrides. Unspecified fields inherit from the rule's defaults.

```yaml
# p is a percentage — unmatched remainder uses rule defaults
chaos:
  - {p: 0.10, reply: {s: 500, b: "error"}}   # 0.1% error
  - {p: 0.05, serve: {drop: 1kb}}            # 0.05% drop
  - {p: 7.00, serve: {span: {speed: 100b/s}}}        # 7% crawl
  # remaining 92.85% normal
```

## Full examples

```bash
# Slow API with concurrency limit
echo '{p: localhost:9999/_mx, b: {
  match: {g: /api/data},
  reply: {s: 200, b: {"items": [1, 2, 3]}},
  serve: {first_byte: 2s, span: 5s, conn: {max: 5, over: {block: 3s, then: {s: 429}}}}
}}' | yurl

# Large download, throttled, drops mid-stream
echo '{p: localhost:9999/_mx, b: {
  match: {_: /download},
  reply: {s: 200, b: {rand!: {size: 10mb, seed: 42}}},
  serve: {span: {speed: 10kb/s}, drop: 2kb}
}}' | yurl

# Flaky auth endpoint
echo '{p: localhost:9999/_mx, b: {
  match: {_: /auth},
  reply: [
    {s: 401, b: "unauthorized"},
    {s: 200, b: "ok"}
  ]
}}' | yurl

# CRUD resource with latency
echo '{p: localhost:9999/_mx, b: {
  match: {_: /toys},
  reply: {crud!: {seed: [
    {id: 1, name: Ball, price: 2.99},
    {id: 3, name: Owl, price: 5.99}
  ]}},
  serve: {first_byte: 200ms}
}}' | yurl

# Mostly fine, occasional errors and slow responses
echo '{p: localhost:9999/_mx, b: {
  match: {_: /api/items},
  reply: {s: 200, b: {items: []}},
  serve: {span: 500ms},
  chaos: [
    {p: 5, reply: {s: 500, b: "internal error"}},
    {p: 3, serve: {span: {speed: 100b/s}}},
    {p: 1, serve: {drop: 512b}}
  ]
}}' | yurl
```

## Request log

```bash
# all recorded requests
echo '{g: localhost:9999/_mx/log}' | yurl

# filter by path
echo '{g: localhost:9999/_mx/log, q: {path: /toys}}' | yurl

# filter by method
echo '{g: localhost:9999/_mx/log, q: {method: POST}}' | yurl

# clear between tests
echo '{d: localhost:9999/_mx/log}' | yurl
```

## Multiple rules

`_mx` accepts a single rule or an array:

```bash
echo '{p: localhost:9999/_mx, b: [
  {match: {_: /a}, reply: {s: 200, b: "a"}},
  {match: {_: /b}, reply: {s: 404}},
  {match: {_: /c}, reply: {s: 200, b: "c"}, serve: {span: 5s}}
]}' | yurl
```

Rules are priority-ordered. Later rules take precedence.

## Tech

Rust, axum, tokio. Single binary.
