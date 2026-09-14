# IPP2MQTT

A print server and status bridge that speaks IPP Everywhere to printers and MQTT to everything else. It is the big brother of [CUPS2MQTT](https://github.com/momentobooth/cups2mqtt), without the CUPS dependency.

Status: in development, built module by module. Discovery is in place.

## Goals

- **MQTT client** — accepts print orders from other systems and publishes printer status, following the CUPS2MQTT message format.
- **IPP client** — talks directly to IPP Everywhere printers, no driver stack.
- **Print server** — takes PDF in and converts it to whatever the printer accepts.
- **Print queue** — persistent, with retries, pause/resume and sane handling of failed jobs.
- **Multi-printer** — one instance serves many printers.
- **Status monitor** — polls printers and publishes changes over MQTT.
- **mDNS client** — finds printers on the network by itself.

No printer specific hacks; only what the IPP and PWG specifications define.

## Current functionality

- [X] mDNS / DNS-SD discovery
  - [X] Browses `_ipp._tcp` and `_ipps._tcp` plus the IPP Everywhere and AirPrint subtypes
  - [X] Builds the printer URI from the `rp` TXT key
  - [X] Reports capabilities from the TXT record (formats, URF, colour, duplex, media, auth, TLS)
  - [X] Deduplicates one printer across service types, subtypes, interfaces and address families
  - [X] Online / stale / offline lifecycle with mDNS reconfirmation
  - [X] Statically configured printers alongside discovered ones
- [X] IPP client
  - [X] Capability probe, cached until the printer reports a configuration change
  - [X] Status polling: state, state reasons with severity, accepting/queued
  - [X] Supply levels from `printer-supply`, including receptacles
  - [X] Job listing and per-job status
  - [X] Print-Job, Create-Job + Send-Document, Validate-Job, Cancel-Job, Identify-Printer
  - [X] Retryable vs permanent error classification
- [ ] MQTT client
- [ ] MQTT messaging and Home Assistant discovery
- [ ] Print queue
- [ ] PDF conversion

## Configuration

Settings are read from `ipp2mqtt.toml` in the working directory, then overridden by environment variables prefixed with `I2M_`. See [ipp2mqtt.example.toml](ipp2mqtt.example.toml).

Because the environment separator is `_`, keys that contain an underscore are written without it in their environment form:

| Setting | Environment variable |
| --- | --- |
| `discovery.enabled` | `I2M_DISCOVERY_ENABLED` |
| `discovery.ignoreinterfaces` | `I2M_DISCOVERY_IGNOREINTERFACES` |
| `discovery.ippeverywhereonly` | `I2M_DISCOVERY_IPPEVERYWHEREONLY` |
| `discovery.staleafter` | `I2M_DISCOVERY_STALEAFTER` |
| `discovery.offlineafter` | `I2M_DISCOVERY_OFFLINEAFTER` |
| `ipp.ignoretlserrors` | `I2M_IPP_IGNORETLSERRORS` |
| `ipp.username` | `I2M_IPP_USERNAME` |
| `ipp.pollinterval` | `I2M_IPP_POLLINTERVAL` |
| `ipp.supplyinterval` | `I2M_IPP_SUPPLYINTERVAL` |
| `ipp.connecttimeout` | `I2M_IPP_CONNECTTIMEOUT` |
| `ipp.polltimeout` | `I2M_IPP_POLLTIMEOUT` |
| `ipp.jobtimeout` | `I2M_IPP_JOBTIMEOUT` |

Log level comes from `RUST_LOG`, for example `RUST_LOG=ipp2mqtt=debug`.

## Usage

```sh
# Browse for printers and report what is found, until interrupted.
ipp2mqtt discover

# Probe printers over IPP and print capabilities, status, supplies and jobs.
ipp2mqtt status
ipp2mqtt status selphy --all-jobs

# Print. Several files use Create-Job and Send-Document instead of Print-Job.
ipp2mqtt print photo.pdf --printer selphy --format application/pdf
ipp2mqtt print a.pdf b.pdf --printer selphy

# Make a printer announce itself, and cancel a job.
ipp2mqtt identify selphy
ipp2mqtt cancel 42 selphy

# Run the service.
ipp2mqtt
```

## IPP notes

**The `ipp` crate is used as a codec, not as a client.** Its `AsyncIppClient` builds a fresh HTTP client on every request, so every poll would cost a new TCP and TLS handshake. ipp2mqtt keeps one pooled connection per printer instead, which measurably holds: five polls of a printer use one TCP connection. It also lets the client set its own connect timeout, speak IPP/2.0, and filter Get-Jobs, none of which the crate's client allows.

**TLS verification is off by default.** Printers ship self-signed certificates and discovery prefers `ipps` where a printer advertises it, so verifying by default would make secure printing unusable out of the box. Set `ipp.ignoretlserrors = false` to require valid certificates.

**Capabilities are fetched once**, then refreshed only when the printer's `printer-config-change-time` moves. The status poll asks for a short list of attributes rather than `all`, which on a real printer is tens of kilobytes per request.

**Three readiness signals are published, not one.** `printer-is-accepting-jobs` governs future jobs and is explicitly independent of `printer-state`, so a stopped printer commonly still queues work. `can_submit`, `will_print_now` and `needs_attention` keep that distinction.

## Discovery notes

**Availability is reported in three states.** `online` means the printer refreshed its mDNS records recently, `stale` means it missed the refresh window and is being reconfirmed, `offline` means it has been quiet past the grace period. Statically configured printers start as `unknown` until they are probed over IPP.

The SRV record TTL is 120 seconds and Wi-Fi printers power-save aggressively, so the defaults are deliberately slow: 2 minutes to stale, 5 minutes to offline. Shortening them makes printers flap.

**Docker needs host networking.** mDNS is multicast with an IP TTL of 1, which does not survive Docker's bridge network; no port mapping fixes this. Use `network_mode: host` on Linux, or a macvlan network. Docker Desktop on macOS and Windows cannot do this at all, because containers run inside a VM with no path to the LAN multicast domain — develop natively there.

**VPN interfaces carry no printers.** If Tailscale or WireGuard is up, list those interfaces in `discovery.ignoreinterfaces`.

## Licence

Mozilla Public License 2.0. See [LICENSE](LICENSE).
