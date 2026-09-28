# delete all routes configured with vpn-up.sh
# and fallback to ISP's default routing
sudo ip route del 0.0.0.0/1 dev wg0
sudo ip route del 128.0.0.0/1 dev wg0
sudo ip route del 187.33.156.69/32