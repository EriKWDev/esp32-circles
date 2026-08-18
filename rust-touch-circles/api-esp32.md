Rainbird irrigation controller - HTTP API for an ESP32 client
App version 0.12.0, running on an AXIS A9210 I/O relay module.

There are exactly two endpoints you need. Both are plain text, one
"key:value" per line - no JSON anywhere.

-----------------------------------------------------------------------
0. CONNECTING
-----------------------------------------------------------------------
Base URL:
    http://<device-ip>/local/rainbird/app/api/...

The app itself listens only on the device's loopback; the Axis device's own
web server reverse-proxies to it and enforces authentication on every
request. So each call needs HTTP DIGEST auth with an admin account on the
device - the same auth any VAPIX call would use. There is no simpler or
token-based mode available, since it is the device's login, not ours.

    curl --digest -u <user>:<pass> \
      "http://<device-ip>/local/rainbird/app/api/dump"

On ESP32: HTTPClient in Arduino-ESP32 supports digest auth via
setAuthorization(user, pass) with newer cores; if yours only does Basic,
esp_http_client (ESP-IDF) has HTTP_AUTH_TYPE_DIGEST. Worth confirming early -
it is the one fiddly part of the integration.

Everything is HTTP/1.0, Connection: close, Cache-Control: no-store.

-----------------------------------------------------------------------
1. GET /api/dump   - full state in one request
-----------------------------------------------------------------------
Read-only, no side effects. ~36 lines, ~450 bytes total. Lines starting
with '#' are comments. Poll it as often as you like (once a second is fine).

Record layout, in the order emitted:

  v:1                                  format version - see "compatibility"
  relays:<n>                           how many relay records follow
  r:<id>:<port>:<enabled>:<on>:<name>  one per relay
  starts:<n>                           how many start times follow
  s:<id>:<enabled>:<hh>:<mm>:<count>   one per start time
  e:<start_id>:<relay_id>:<seconds>    one per schedule entry, in RUN ORDER
  time:<HH:MM:SS>                      device's local clock
  run:<0|1>                            is any relay energized right now
  active:<relay_id>                    0 = idle, -1 = raw test pulse
  left:<seconds>                       remaining on the active run
  queued:<n>                           entries still to come in a sequence
  qgap:<0|1>                           currently in the pause between relays
  maxrun:<seconds>                     server-side clamp on any single run
  relaygap:<seconds>                   configured pause between relays
  analogs:<n>                          how many analog records follow
  a:<port>:<level>:<name>              one per readable analog input
  err:<text>                           empty string when there is no error

Field notes:
  - <enabled> is config ("is this relay in use"), <on> is live ("is it
    energized right now"). At most ONE relay ever has <on>=1 - see section 3.
  - <port> is the physical VAPIX I/O port. You do not need it to control
    anything; it is there for wiring diagnostics.
  - The relay records come FIRST, so every relay id referenced later
    (active:, e:) already has a name on record by the time you read it.
  - <hh>/<mm> are separate integers rather than "HH:MM" specifically so one
    sscanf can take the whole record with no colon inside a value.
  - <start_id> is repeated on every e: line, so you can parse entries
    statelessly without tracking which s: line came before.
  - There are SEVERAL analog inputs, not one. The a: records are
    supervised-capable digital inputs repurposed to read external analog
    sensors, and <level> is the raw value the device reports - Axis does not
    document what 4095 corresponds to in volts, so nothing is derived from it
    here. <port> is the device's own input port id and is the stable
    identifier; <name> is the terminal marking ("I1", "I/O 1" - note it can
    contain spaces and a slash, which is fine since it's the last field).
  - Only inputs with a VALID reading appear. On this hardware that is the
    master's own 7 inputs; the A9910 extension's 8 I/Os answer with an
    0xFFFFFFFF "no reading" sentinel and are omitted. So analogs:<n> tells
    you how many you actually got, and a sensor is never reported as a fake
    0 - which would be indistinguishable from a real sensor reading zero.
    If the whole read fails you get analogs:0 and no a: lines.

Real example (5 relays, 1 start time with 7 entries, idle):

  # rainbird dump - read-only; not the /api/import format
  v:1
  relays:5
  r:1:1:1:0:dammen
  r:2:20:1:0:gräsmatta
  r:3:21:1:0:grannen
  r:4:22:1:0:mot vägen
  r:5:23:1:0:mot havet
  starts:3
  s:1:1:11:10:7
  e:1:1:2
  e:1:2:2
  e:1:3:2
  e:1:5:2
  e:1:4:2
  e:1:1:1
  e:1:1:1
  s:2:0:12:00:0
  s:3:0:21:00:0
  time:13:47:43
  run:0
  active:0
  left:0
  queued:0
  qgap:0
  maxrun:3600
  relaygap:0
  analogs:7
  a:1:1:I1
  a:2:1018:I2
  a:5:1016:I3
  a:7:744:I/O 1
  a:8:746:I/O 2
  a:9:1013:I4
  a:10:1017:I5
  err:

Parsing it (this is the whole parser):

  // one line at a time; ignore anything you don't recognise
  int id, port, en, on, hh, mm, cnt, sid, rid, secs, v, lvl;

  if (sscanf(line, "v:%d", &v) == 1) { /* check compatibility */ }
  else if (sscanf(line, "r:%d:%d:%d:%d:", &id,&port,&en,&on) == 4) {
      const char *name = nth_colon(line, 5);   // free text = rest of line
  }
  else if (sscanf(line, "s:%d:%d:%d:%d:%d", &sid,&en,&hh,&mm,&cnt) == 5) { }
  else if (sscanf(line, "e:%d:%d:%d", &sid,&rid,&secs) == 3) { }
  else if (sscanf(line, "a:%d:%d:", &port,&lvl) == 2) {
      const char *name = nth_colon(line, 3);   // free text = rest of line
  }
  else if (sscanf(line, "run:%d", &on) == 1) { }
  else if (sscanf(line, "active:%d", &id) == 1) { }
  else if (sscanf(line, "left:%d", &secs) == 1) { }
  // ...etc

TEXT FIELDS: relay names, analog names and err: are free text and are ALWAYS
the last field on their line. So: split on the first N colons and take the
remainder verbatim. A ':' inside a relay name therefore needs no escaping
and must not be treated as a separator. CR/LF inside text is folded to
spaces by the server, so a record can never span two lines - you can safely
treat "one line = one record". Names are UTF-8 (note "gräsmatta" above);
they are bytes to you, no decoding needed unless you render them.

COMPATIBILITY: check v:. New key names may be added in future versions, so
ignore lines you don't recognise rather than erroring - that way a firmware
built today keeps working against a newer app.

-----------------------------------------------------------------------
2. GET /api/trigger?relay=<id>&seconds=<n>   - turn a relay on
-----------------------------------------------------------------------
Turns relay <id> on for <n> seconds.

"Last request wins": if anything else is running - another relay, or a
multi-relay schedule sequence - it is dropped and this request takes over.
That happens atomically, so two controllers racing each other can never end
up with both relays energized; one of them simply wins. This is the opposite
of POST /api/run, which refuses when a different relay is active (that one
exists for the web UI, to catch accidental double-clicks).

<n> is silently clamped to maxrun (see dump) if you ask for more.

Success (HTTP 200):
    ok:1
    active_relay:3
    remaining_sec:20

Failure (HTTP 400 bad/missing params, 409 unknown or disabled relay):
    ok:0
    error:<text>

Verified responses:
    (no params)              -> HTTP 400  ok:0  error:missing relay or seconds query parameter
    ?relay=99&seconds=5      -> HTTP 409  ok:0  error:okänt relä-id 99
    ?relay=1&seconds=99999   -> HTTP 200  ok:1  active_relay:1  remaining_sec:3599  (clamped to maxrun)

Heads-up: error TEXT is in Swedish (the app's UI language) - "okänt relä-id"
= "unknown relay id", "är avstängt i konfigen" = "is disabled in config".
Branch on the HTTP status code and on ok:0/ok:1, not on the message text.

IMPORTANT - "ok:1" means ACCEPTED, NOT COMPLETED. The call returns in about
0.1s and does not wait out the <n> seconds. It also cannot confirm the relay
physically pulled: the reason is in section 4. If the device later refuses
the drive (unreachable, unknown port), that shows up as err: in /api/dump,
not in this reply. So: to confirm a run is actually happening, poll
/api/dump and look at run:/active:/left:/err: - don't infer it from ok:1.

To stop early, either trigger something else, or POST /api/stop (no body).

-----------------------------------------------------------------------
3. THE ONE HARD RULE
-----------------------------------------------------------------------
At most ONE relay is ever energized, across every configured device. The app
enforces this itself - you cannot break it from the API side, and you don't
need to coordinate to avoid it. Verified by firing six concurrent
conflicting triggers and watching the physical port states: never more than
one closed at any moment.

Practical consequence for your firmware: you do NOT need to send an "off"
before switching zones. Just trigger the next relay; the app drops the
current one first.

-----------------------------------------------------------------------
4. WHY THE API IS SHAPED THIS WAY (device quirk worth knowing)
-----------------------------------------------------------------------
Relays are never latched on. Each run is armed as a single device-side timed
pulse covering the whole run, so the DEVICE drops the relay even if the app
dies - a valve can't be left open indefinitely. The app also sends its own
OFF at the requested second, and normally that's what ends a run.

The quirk: the A9210's io/port.cgi does NOT return when a pulse is armed. It
holds the HTTP response open until the pulse ENDS - a 15s pulse takes 15s to
answer. (It returns early if something else ends the pulse, and a new arm
resets the port's timer rather than queueing.) This is measured, not
documented by Axis.

That is why /api/trigger answers "accepted" rather than "done": waiting for
the drive call to return would make the HTTP request take as long as the
watering itself. Nothing for you to handle - just don't expect ok:1 to mean
the relay is confirmed, and poll /api/dump instead.

-----------------------------------------------------------------------
5. THE REST OF THE API (JSON; you should not need any of it)
-----------------------------------------------------------------------
  GET  /api/status         engine state + analog, JSON
  GET  /api/config         full config, JSON, passwords masked
  POST /api/config         validate, save atomically, hot-reload
  POST /api/run            {"relay":N,"seconds":S} - refuses if another
                            relay is active (unlike /api/trigger)
  POST /api/run-sequence   {"entries":[{"relay":N,"seconds":S},...]}
                            runs relays one after another
  POST /api/test-port      {"device":"master","port":N,"seconds":S}
                            raw port pulse, bypasses the relay config
  POST /api/stop           stop whatever is active or queued
  GET  /api/ports          raw VAPIX IOPort listing
  GET  /api/export         relay names + schedule as editable text
  POST /api/import         same text format back (NOT the /api/dump format)

Note /api/export and /api/dump are different formats for different jobs:
export is a hand-editable config backup, dump is a machine-readable state
snapshot and is read-only.

-----------------------------------------------------------------------
Happy to walk through the digest-auth setup if that turns out to be the
annoying part, or to add fields to the dump if something you need is
missing - adding keys is backwards-compatible by design.
