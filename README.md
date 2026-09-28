# wgctl

`wgctl` is an experimental Linux WireGuard management tool written in Rust. It creates and configures a WireGuard interface by communicating directly with the Linux kernel:

- **rtnetlink** creates the interface, assigns its address, changes its state, and sets its MTU.
- **Generic Netlink** sends WireGuard-specific configuration such as the private key, listening port, peers, endpoints, allowed IPs, and persistent keepalive.

This project is currently a learning prototype. My idea is to learn netlink communication and linux routing. In particular, it configures the WireGuard device and automatically install all routes on peer, it doesn't configure IP forwarding, configure NAT, or safely roll back on VPS.

## 1. How the pieces fit together

A working Internet VPN has more than a WireGuard interface:

```text
Client application
    |
    | creates wg0 and configures a peer
    v
Client wg0 (10.50.0.2)
    |
    | encrypted UDP, usually port 51820
    v
VPS wg0 (10.50.0.1)
    |
    | IPv4 forwarding + nftables masquerade
    v
VPS public interface
    |
    v
Internet
```

WireGuard encrypts and decrypts packets, but it is not a routing daemon or firewall manager. Linux routes decide which packets enter `wg0`; the VPS must forward those packets and normally translate their private source addresses with NAT.

## 2. Requirements

- Linux with WireGuard kernel support
- Rust toolchain and Cargo
- Root privileges or the necessary network capabilities
- `iproute2` for the `ip` diagnostic command
- `wireguard-tools` for the optional `wg` diagnostic command
- `nftables` on the VPS when it acts as an Internet gateway

Install it according to you linux distribution and packet manager.

## 3. Build

```bash
cargo build
```

Because creating interfaces and configuring WireGuard requires elevated privileges, run the current prototype as root:

```bash
sudo cargo run
```

The program currently reads `./wgctl.toml` configuration.

## 4. Keys and peer identity

Every WireGuard device has one private key and one corresponding public key. Peers must have different key pairs:

```text
Client private key  -> remains only on the client
Client public key   -> configured as a peer on the VPS

VPS private key     -> remains only on the VPS
VPS public key      -> configured as a peer on the client
```

WireGuard keys are raw Curve25519/X25519 keys encoded with Base64. They are not certificates: there is no certificate authority, expiry date, hostname, or identity document attached to them.

Suggestion: never copy a private key into a peer's `public_key` field and never share a private key. The current application expects its own Base64 private key in the file named by `private_key_path`. Application can create key pairs with create-keys command i.e.
```bash
sudo wgctl create-keys
```

Keep keys safe, dont copy private to anyone, generate it only on peers, public key is safe to copy from peer to peer.

Protect private-key files:

```bash
chmod 600 private.key
```

## 5. Configuration file

The current configuration format is TOML. A client configuration looks like this:

```toml
[interface]
name = "wg0"
address = "10.50.0.2/24"
private_key_path = "private.key"
mtu = 1420
port = 51820

[[peer]]
peer_name = "SOMETHING TO DISTRINGUSIH PEER"
public_key = "VPS_PUBLIC_KEY_BASE64"
endpoint = "VPS_PUBLIC_IP:51820"
allowed_ips = ["10.50.0.1/32"]
persistent_keepalive = 25
```

The fields mean:

| Field | Meaning |
|---|---|
| `name` | Linux interface name, for example `wg0` |
| `address` | This device's tunnel address and prefix |
| `private_key_path` | Path to this device's private key |
| `mtu` | Maximum packet size on the tunnel; `1420` is a common starting point |
| `port` | Local WireGuard UDP listening port |
| `public_key` | The remote peer's public key |
| `endpoint` | The remote peer's reachable public IP and UDP port |
| `allowed_ips` | Addresses that may be received from this peer and destinations sent to it |
| `persistent_keepalive` | Periodic authenticated packet interval; `25` seconds is useful behind NAT/CGNAT |

### Understanding `AllowedIPs`

`AllowedIPs` has two connected jobs:

1. On transmission, WireGuard uses it to choose the peer for a destination.
2. On reception, WireGuard uses it to verify which source addresses that peer may send.

For a tunnel-only test, the client should use the VPS tunnel address:

```toml
allowed_ips = ["10.50.0.1/32"]
```

For an IPv4 full tunnel, the client uses:

```toml
allowed_ips = ["0.0.0.0/0"]
```

On the VPS, the peer representing this client should normally have:

```text
AllowedIPs = 10.50.0.2/32
```

## 6. Configure the VPS

The following commands belong on the Clouding VPS (or any other provider), not on the client PC.

### 1. Open the cloud firewall

In the Clouding control panel, allow inbound **UDP port 51820** for the VPS. On Clouding.io it is enought to add it in default configuration. A provider firewall exists outside the VM, so it is separate from `nftables`, UFW, or firewalld inside Linux.

Keep the SSH rule that allows you to administer the server.

Suggestion: keep all not used ports closed.

### 2. Find the public network interface on your VPS

Find public network interface using:

```bash
ip route show default
```

Possible output:

```text
default via 192.0.2.1 dev eth0
```

In that example the public interface is `eth0`. Substitute the actual interface name in every rule below.

### 3. Enable IPv4 forwarding

Enable it:

```bash
sudo sysctl -w net.ipv4.ip_forward=1
```

If you wish make it persistent by placing this line in `/etc/sysctl.d/90-wgctl-forwarding.conf`:

```text
net.ipv4.ip_forward=1
```

Then load it and verify it:

```bash
sudo sysctl --system
sysctl net.ipv4.ip_forward
```

The result should be `net.ipv4.ip_forward = 1`.

### 4. Configure forwarding and NAT with nftables

The following example assumes:

- WireGuard interface: `wg0`
- Tunnel subnet: `10.50.0.0/24`
- VPS public interface: `eth0`

Put these tables in `/etc/nftables.conf`, changing `eth0` if necessary:

```nft
table inet wgctl_filter {
    chain forward {
        type filter hook forward priority filter;
        policy drop;

        iifname "wg0" oifname "eth0" accept
        iifname "eth0" oifname "wg0" ct state established,related accept
    }
}

table ip wgctl_nat {
    chain postrouting {
        type nat hook postrouting priority srcnat;
        policy accept;

        ip saddr 10.50.0.0/24 oifname "eth0" masquerade
    }
}
```

The first forwarding rule permits client traffic to leave through the VPS. The second permits only reply traffic back toward WireGuard. `masquerade` changes the private tunnel source address to the VPS public address so Internet hosts can reply.

The forwarding chain has a drop policy. If the VPS already routes other networks, add their required forwarding rules too before loading this ruleset.

Suggestion: check syntax before applying it:

```bash
sudo nft --check --file /etc/nftables.conf
```

No output means the syntax check succeeded. `--check` does not apply the file. Apply and enable it with:

```bash
sudo systemctl enable --now nftables
sudo systemctl reload nftables
```

Verify the active rules:

```bash
sudo nft list ruleset
```

Check if UFW or firewalld is already managing the host firewall, Determine which service owns the ruleset first:

```bash
sudo ufw status
sudo systemctl is-active firewalld
sudo systemctl is-active nftables
```
Disable the one you dont want to manage firewall.

## 7. Verify the basic tunnel

After configuring both peers, check the interface:

```bash
ip -details link show dev wg0
ip address show dev wg0
sudo wg show wg0
```

A WireGuard link commonly displays `state UNKNOWN`. That is normal: it is a virtual point-to-point interface with no physical carrier whose up/down status Linux could detect. The important indicators are the `UP` flag, a recent handshake, and increasing transfer counters.

TesTry to ping the tunnel addresses.

client -> VPS:

```bash
ping 10.50.0.1
```

VPS -> client:

```bash
ping 10.50.0.2
```

A recent handshake proves that encrypted WireGuard packets are being exchanged. It does not by itself prove that routes, forwarding, NAT, or firewall policy are correct.

## 8. Test Internet forwarding safely

Before redirecting all client traffic, check it with single public address through the VPN:
Add rule for routing 1.1.1.1/32 through wireguard link which is in this case wg0.
Delete it after testing!

```bash
sudo ip route add 1.1.1.1/32 dev wg0
ping 1.1.1.1
sudo ip route del 1.1.1.1/32 dev wg0
```

If this passes without errors, the full path is working:

```text
client route -> WireGuard -> VPS forwarding -> NAT -> Internet
```

If it fails try to diagnose where it stucked.

## 9. Route all IPv4 traffic through the VPN

These commands to client peer.

First configure the VPS peer on the client with:

```toml
allowed_ips = ["0.0.0.0/0"]
```

Then preserve a direct route to the WireGuard endpoint. Without this route, the encrypted WireGuard UDP packets could themselves be sent into `wg0`, creating a routing loop.

Inspect the existing path to the VPS public IP:

```bash
ip route get <VPS_IP>
```

Suppose it reports gateway `192.168.0.1` and device `wlan0`. Add a host route using those actual values:

```bash
sudo ip route add <VPS_IP>/<CIDR> via 192.168.0.1 dev wlan0
```

Now add two routes (default splitting) that together cover all IPv4 destinations:

```bash
sudo ip route add 0.0.0.0/1 dev wg0
sudo ip route add 128.0.0.0/1 dev wg0
```
These two `/1` routes are more specific than the ordinary `/0` default route. The VPS endpoint's `/32` route is even more specific, so encrypted transport packets continue to use the physical network.

All that can be done using app:
```bash
sudo sudo wgctl up --peer-name <PEER_NAME> --interface wlan0 --interface-ip 192.168.0.1/32
```

Verify routing and the visible public address:

```bash
ip route get 1.1.1.1
curl -4 https://ifconfig.me
sudo wg show wg0
```

The route lookup should select `wg0`, and the public address should be the VPS address.

To undo the full-tunnel routes:

```bash
sudo ip route del 0.0.0.0/1 dev wg0
sudo ip route del 128.0.0.0/1 dev wg0
sudo ip route del <VPS_IP>/<CIDR>
```

Or using app with:
```bash
sudo sudo wgctl down --peer-name <PEER_NAME> --interface wlan0 --interface-ip 192.168.0.1/32
```

Only delete the endpoint route if it was added specifically for this test.

### IPv6 warning

The configuration above tunnels IPv4 only. If the client has working IPv6, IPv6 traffic may bypass the VPN. A complete implementation must either configure an IPv6 tunnel and routes or deliberately block IPv6 while full-tunnel mode is active.

## 10. Troubleshooting & checks

### No handshake

Check:

- The client has the VPS public key, and the VPS has the client public key.
- Neither side is using the other side's private key!!!
- The endpoint contains the VPS public IP and correct UDP port.
- VPS allows inbound UDP port 51820.
- WireGuard is listening on the expected port.
- A NATed or CGNAT client uses `persistent_keepalive = 25`.

Useful commands:

```bash
sudo wg show
sudo ss -lunp | grep 51820
```

### Handshake works, but tunnel addresses cannot be pinged

Check the tunnel addresses and peer `AllowedIPs`. For this example:

```text
Client address:              10.50.0.2/24
Client's VPS AllowedIPs:     10.50.0.1/32 (or 0.0.0.0/0 for full tunnel)
VPS address:                 10.50.0.1/24
VPS's client AllowedIPs:     10.50.0.2/32
```

Also check host firewall input policy and whether transfer counters increase during a ping.

### Tunnel ping works, but Internet access does not

On the VPS, verify:

```bash
sysctl net.ipv4.ip_forward
sudo nft list ruleset
ip route show default
```

Confirm that the nftables public-interface name matches the interface in the default route. During a test, packet capture can show where traffic stops:

```bash
sudo tcpdump -ni wg0
sudo tcpdump -ni eth0
```

Replace `eth0` with the actual public interface.

### Internet fails immediately after adding full-tunnel routes

Check that the endpoint has a more-specific route over the physical network:

```bash
ip route get <VPS_IP>
```

It must not resolve through `wg0`.

### `RTNETLINK answers: File exists`

The route or interface probably already exists. Inspect current state instead of repeatedly adding it:

```bash
ip link show wg0
ip route show
```

### Interface shows `UNKNOWN`

This is expected for WireGuard and does not mean the tunnel is broken. Use handshakes, transfer counters, route lookup, and actual traffic tests as health indicators.

## 11. Current prototype limitations

The present source has several assumptions worth understanding before relying on it operationally:

- It always tries to add the interface, so running it again while `wg0` exists can fail.
- It reads only `./wgctl.toml`; there is no CLI argument yet.
- `mtu`, peer `endpoint`, and `persistent_keepalive` are represented as optional in Rust but are currently unwrapped during execution.
- Allowed-IP encoding currently declares IPv4, so IPv6 configuration is not implemented.
- Full-tunnel routes, endpoint-route protection, forwarding, NAT, DNS, teardown, and rollback are not managed by the application.
- Private-key file creation does not currently enforce restrictive permissions by itself.
- Some parsing and Netlink response paths still use `unwrap`, which can panic instead of returning a helpful `anyhow::Result` error.
- Configuration can replace or add device state, but lifecycle behavior and peer removal need explicit design.

## 12. Steps to bring VPN up
- configure VPS server as mentioned in point 6 (open port, configure nftable, enable forwarding as mentioned before)
- create wgctl.toml with correct data and place it in same directoy as wgctl
- first time create private and public keys using:
```bash
sudo wgctl create-keys
```
- create interfaces using:
```bash
sudo wgctl create
```
- bring VPN up
```bash
sudo wgctl up --peer-name <PEER_NAME> --interface <INTERFACE_NAME> --interface-ip <INTERFACE_IP>/<CIDR>
```
- later to bring it down
```bash
sudo wgctl down --peer-name <PEER_NAME> --interface <INTERFACE_NAME> --interface-ip <INTERFACE_IP>/<CIDR>
```

Since the tool is a learning project dont rely on it and inspect its output with `ip`, `wg`, and `nft` after each configuration step.

## 13. Security notes

- Store private keys with mode `0600` and never commit them.
- Give every device its own key pair.
- Restrict VPS firewall rules to ports that are actually needed.
- Keep an active SSH session while changing remote firewall rules so mistakes can be corrected.
- Validate the endpoint route before enabling a full tunnel.
- Decide explicitly how DNS and IPv6 should behave; otherwise either can leak outside an IPv4-only tunnel.
- Avoid logging private keys or placing them in command histories.

## 14. Quick verification checklist

On the client:

```bash
ip address show dev wg0
sudo wg show wg0
ip route get 10.50.0.1
ip route get 1.1.1.1
```

On the VPS:

```bash
ip address show dev wg0
sudo wg show wg0
sysctl net.ipv4.ip_forward
sudo nft list ruleset
ip route show default
```

A healthy full tunnel has a recent handshake, increasing send/receive counters, a client Internet route through `wg0`, forwarding enabled on the VPS, and matching nftables forwarding/NAT rules.
