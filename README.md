# mockinx - compliant web server, broken on purpose

Configure endpoint behavior via API, and get back exactly that: all fine, CRUD,
slow streams, mid-connection drops, throttled responses, and more.

## Usage

```bash
mockinx -p 9999 -c stubs.yaml     # Start server, read instructions from file
```

```bash
# Configure an endpoint
echo '{p: localhost:9999/_mx, b: {
  match: {g: /toys/3},
  reply: {s: 200, h: {ct!: j!}, b: {name: Owl, price: 5.99}}
}}' | yurl

# Hit it
echo '{g: localhost:9999/toys/3}' | yurl
# => {"name": "Owl", "price": 5.99}
```

## Stub format

A stub has up to four sections:

```yaml
match: {}        # which requests to match
reply: {}     # status, headers, body (yttp {s: h: b:} convention)
delivery: {}     # how bytes hit the wire
behavior: {}     # endpoint-level policies
```

### match

```yaml
match: {g: /api/data}        # GET /api/data
match: {p: /api/data}        # POST /api/data
match: {_: /api/data}        # any method, /api/data
match: _                     # match everything
```

### reply

Uses the yttp `{s: h: b:}` convention for status, headers, body. Header shortcuts (from yttp) are expanded:

```yaml
# simple
reply: {s: 200, h: {ct!: t!}, b: "hello"}

# status only
reply: {s: 204}

# json
reply: {s: 200, h: {ct!: j!}, b: {"items": [1, 2, 3]}}

# generated body
reply: {s: 200, b: {rand: {size: 10kb, seed: 7}}}
reply: {s: 200, b: {pattern: {repeat: "abc", size: 1mb}}}

# wrong content-type (malformed response)
reply: {s: 200, h: {ct!: h!}, b: '{"valid": "json"}'}
```

### delivery

How the response is delivered on the wire:

```yaml
delivery:
  duration: 5s            # spread body over this time
  speed: 10kb/s           # bandwidth cap
  drop: {after: 2kb}      # kill connection after N bytes
  drop: {after: 1s}       # kill connection after N time
  first_byte: {delay: 2s} # delay before first byte
  chunk: {size: 1kb, delay: 100ms}  # chunked streaming

# any scalar value supports jitter via ranges
# explicit range:  min..max
# percentage:      value..percent
delivery:
  duration: 4s..6s                  # uniform random between 4s and 6s
  speed: 10kb/s..20%                # 8kb/s..12kb/s
  drop: {after: 1kb..4kb}
  first_byte: {delay: 1s..10%}     # 900ms..1.1s
  chunk: {size: 512b..2kb, delay: 50ms..150ms}

  # probabilistic — pick one delivery profile per request
  pick:
    - {p: 0.9}                        # normal
    - {p: 0.05, drop: {after: 2kb}}   # connection drop
    - {p: 0.05, speed: 100b/s}        # crawl
```

### behavior

Endpoint-level policies that decide whether/when a request gets served:

```yaml
behavior:
  # concurrency — reject, block, or block with timeout
  concurrency: {max: 5, over: {s: 429, b: "too many"}}
  concurrency: {max: 5, over: block}
  concurrency: {max: 5, over: {block: 3s, then: {s: 429, b: "timeout"}}}

  # rate limit
  rate_limit: {rps: 100, over: {s: 429}}

  # random failures — 10% of requests get error reply
  fail: {rate: 0.1, reply: {s: 500, b: "internal error"}}

  # max request lifetime
  timeout: 30s

  # sequences — different reply per call, counter resets per connection or per stub
  sequence:
    per: connection
    replies:
      - {s: 401, b: "unauthorized"}
      - {s: 200, b: "ok"}

  # crud — in-memory REST resource
  crud:
    id: {name: id, new: auto}     # default
    seed:
      - {id: 1, name: Ball, price: 2.99}
      - {id: 3, name: Owl, price: 5.99}
```

## Full examples

```bash
# Slow API with concurrency limit
echo '{p: localhost:9999/_mx, b: {
  match: {g: /api/data},
  reply: {s: 200, h: {ct!: j!}, b: {"items": [1, 2, 3]}},
  delivery: {first_byte: {delay: 2s}, duration: 5s},
  behavior: {concurrency: {max: 5, over: {block: 3s, then: {s: 429}}}}
}}' | yurl

# Large download, throttled, drops mid-stream
echo '{p: localhost:9999/_mx, b: {
  match: {_: /download},
  reply: {s: 200, b: {rand: {size: 10mb, seed: 42}}},
  delivery: {speed: 10kb/s, drop: {after: 2kb}}
}}' | yurl

# Flaky auth endpoint
echo '{p: localhost:9999/_mx, b: {
  match: {_: /auth},
  behavior: {sequence: [
    {s: 401, b: "unauthorized"},
    {s: 200, b: "ok"}
  ]}
}}' | yurl

# CRUD resource with latency
echo '{p: localhost:9999/_mx, b: {
  match: {_: /toys},
  reply: {h: {ct!: j!}},
  delivery: {first_byte: {delay: 200ms}},
  behavior: {crud: {seed: [
    {id: 1, name: Ball, price: 2.99},
    {id: 3, name: Owl, price: 5.99}
  ]}}
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

## Multiple stubs

`_mx` accepts a single object or an array:

```bash
echo '{p: localhost:9999/_mx, b: [
  {match: {_: /a}, reply: {s: 200, b: "a"}},
  {match: {_: /b}, reply: {s: 404}},
  {match: {_: /c}, reply: {s: 200, b: "c"}, delivery: {duration: 5s}}
]}' | yurl
```

Stubs are priority-ordered. Later stubs take precedence.

## Tech

Rust, axum, tokio. Single binary.
