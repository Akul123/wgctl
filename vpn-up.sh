sudo ip route add 187.33.156.69/32 via 192.168.100.1 dev wlan0

# default-route splitting so that VPN doesn't mess up ISP (default) routing
sudo ip route add 0.0.0.0/1 dev wg0
sudo ip route add 128.0.0.0/1 dev wg0