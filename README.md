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
- [ ] IPP client
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

Log level comes from `RUST_LOG`, for example `RUST_LOG=ipp2mqtt=debug`.

## Usage

```sh
# Browse for printers and report what is found, until interrupted.
ipp2mqtt discover

# Run the service.
ipp2mqtt
```

## Discovery notes

**Availability is reported in three states.** `online` means the printer refreshed its mDNS records recently, `stale` means it missed the refresh window and is being reconfirmed, `offline` means it has been quiet past the grace period. Statically configured printers start as `unknown` until they are probed over IPP.

The SRV record TTL is 120 seconds and Wi-Fi printers power-save aggressively, so the defaults are deliberately slow: 2 minutes to stale, 5 minutes to offline. Shortening them makes printers flap.

**Docker needs host networking.** mDNS is multicast with an IP TTL of 1, which does not survive Docker's bridge network; no port mapping fixes this. Use `network_mode: host` on Linux, or a macvlan network. Docker Desktop on macOS and Windows cannot do this at all, because containers run inside a VM with no path to the LAN multicast domain — develop natively there.

**VPN interfaces carry no printers.** If Tailscale or WireGuard is up, list those interfaces in `discovery.ignoreinterfaces`.

## Licence

Mozilla Public License 2.0. See [LICENSE](LICENSE).
