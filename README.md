# mockinx - compliant web server, broken on purpose

\[MO-keen-x\]: `match` + `reply` + `serve` + `chaos` = what you get back.

Codeless, easy config: all good · CRUD · parallel · slow · trickle · drops · errors

Rules use [yttp](https://crates.io/crates/yttp) conventions for HTTP requests and responses.

```bash
cargo install mockinx yurl    # yurl instead of curl
mockinx                       # start server on port 9999
mockinx 9998 -c rules.yaml    # listen on 9998, rules from file
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
serve: {}   # how to serve it   (pace, drop, limits)
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
reply: {crud!: {data: [{id: 1, name: Ball}, {id: 3, name: Owl}]}}
reply: {crud!: {id: {name: sku, new: inc}}}         # auto-increment IDs
reply: {crud!: {id: {name: uid, new: uuid}}}        # UUID IDs
reply: {crud!: true}                                # no data, id: "id", inc
```

### serve

How the response is served — delivery shaping and operational constraints:

```yaml
serve:
  # delivery shaping
  pace: 5s             # pacing: duration target (auto-chunked)
  pace: 1kb@100ms      # pacing: 1kb chunks at every 100ms
  pace: 10kb/s         # pacing: bandwidth cap

  drop: 2kb            # kill connection after N bytes
  drop: 1s             # kill connection after N time
  
  first_byte: 2s       # time to first byte (delay)

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
  pace: 4s..6s                 # random timespan
  pace: 512b..2kb@50ms..150ms  # random chunk size and sending interval
  pace: 10kb/s..20%            # random bandwidth 8kb/s..12kb/s
  drop: 1kb..4kb               # drop conn anywhere in that byte range
  first_byte: 1s..10%          # 900ms..1.1s
```

### chaos

Probabilistic overrides for reply and/or serve. Each entry has a percentage `p`
and `reply`/`serve` overrides. Unspecified fields inherit from the rule's defaults.

```yaml
# p is a percentage — unmatched remainder uses rule defaults
chaos:
  - {p: 0.10%, reply: {s: 500, b: "error"}}   # 0.1% error
  - {p: 0.05%, serve: {drop: 1kb}}            # 0.05% drop after 1kb
  - {p: 7.00%, serve: {pace: 100b/s}}         # 7% crawl
  # remaining 92.85% normal (0 padding added for readability)
```

## Examples

```bash
# Slow API with concurrency limit
echo '{p: localhost:9999/_mx, b: {
  match: {g: /api/data},
  reply: {s: 200, b: {"items": [1, 2, 3]}},
  serve: {first_byte: 2s, pace: 5s, conn: {max: 5, over: {block: 3s, then: {s: 429}}}}
}}' | yurl

# Large download, throttled, drops mid-stream
echo '{p: localhost:9999/_mx, b: {
  match: {_: /download},
  reply: {s: 200, b: {rand!: {size: 10mb, seed: 42}}},
  serve: {pace: 10kb/s, drop: 2kb}
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
  reply: {crud!: {data: [
    {id: 1, name: Ball, price: 2.99},
    {id: 3, name: Owl, price: 5.99}
  ]}},
  serve: {first_byte: 200ms}
}}' | yurl

# Mostly fine, occasional errors and slow responses
echo '{p: localhost:9999/_mx, b: {
  match: {_: /api/items},
  reply: {s: 200, b: {items: []}},
  serve: {pace: 500ms},
  chaos: [
    {p: 5%, reply: {s: 500, b: "internal error"}},
    {p: 3%, serve: {pace: 100b/s}},
    {p: 1%, serve: {drop: 512b}}
  ]
}}' | yurl
```

## Managing rules: /_mx

```bash
# POST — append rules (single or array)
echo '{p: localhost:9999/_mx, b: [
  {match: {_: /a}, reply: {s: 200, b: "a"}},
  {match: {_: /b}, reply: {s: 404}}
]}' | yurl

# GET — list active rules (most recent first)
echo '{g: localhost:9999/_mx}' | yurl

# PUT — replace all rules (atomic reset + load)
echo '{put: localhost:9999/_mx, b: [
  {match: {_: /new}, reply: {s: 200, b: "fresh start"}}
]}' | yurl

# PUT with empty array — clear all rules
echo '{put: localhost:9999/_mx, b: []}' | yurl
```

Rules are priority-ordered. Later rules take precedence.

## Managing rules: config file

```yaml
# rules.yaml — load with: mockinx 9999 -c rules.yaml
# see: ./tests/fixtures/rules.yaml
# equivalent to posting at /_mx - see yurl examples above.

# simple static reply
- match: {g: /health}
  reply: {s: 200, b: ok}

# CRUD resource
- match: {_: /toys}
  reply:
    crud!:
      data:
        - {id: 1, name: Ball, price: 2.99}
        - {id: 3, name: Owl, price: 5.99}

# slow endpoint with concurrency limit
- match: {g: /api/data}
  reply: {s: 200, b: {"items": [1, 2, 3]}}
  serve: {pace: 2s, conn: {max: 3, over: {s: 429}}}

# /toys/6 is flaky — overrides the CRUD rule (later = higher priority)
- match: {g: /toys/6}
  reply: {s: 200, b: {id: 6, name: Dice, price: 0.99}}
  serve: {pace: 3s}
  chaos:
    - {p: 30%, reply: {s: 500, b: "oops"}}
    - {p: 10%, serve: {drop: 100b}}
```

Load with: `mockinx -c rules.yaml`

## Performance

Zero overhead — mockinx matches raw axum throughput:

```
                       req/sec    latency
raw TCP (no parsing)   194k       496µs
axum (baseline)        191k       502µs
mockinx (rule match)   190k       499µs
```

Run `./benches/run.sh` (requires [wrk](https://github.com/wg/wrk)).

## Tech

Rust, axum, tokio. Single binary.
