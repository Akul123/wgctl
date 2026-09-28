use anyhow::anyhow;
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures_util::stream::TryStreamExt;
use ipnetwork::{IpNetwork, Ipv4Network};
use netlink_packet_route::link::{InfoKind, LinkAttribute, LinkInfo, LinkMessage};
use rtnetlink::{Handle, LinkUnspec, RouteMessageBuilder, new_connection as rtnetlink_connection};
use std::fs::File;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::str::FromStr;

use genetlink::new_connection as genetlink_connection;

use crate::wireguard_device::WireGuardDevice;
use crate::wireguard_key_pair::WireguardKeyPair;

mod wireguard_config;
mod wireguard_device;
mod wireguard_key_pair;

#[derive(Parser)]
#[command(
    name = "wgctl",
    about = "Experimental Linux WireGuard management tool",
    version = "0.1.0"
)]

struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Clone)]
enum Command {
    Create,         // Add ip link and wireguard device
    Up(UpArgs),     // bring VPN up
    Down(DownArgs), // bring VPN down
    Delete,         // Delete ip link
    CreateKeys,     // Create key pairs
    Show,           // show info about peers
}

#[derive(Subcommand, Clone, PartialEq, Eq, Hash, ValueEnum, Debug)]
enum SubCommand {
    UpArgs,
    DownArgs,
}

#[derive(Args, Clone)]
struct UpArgs {
    #[arg(short, long)]
    peer_name: String,
    #[arg(short = 'i', long)]
    interface: String,
    #[arg(short = 'a', long)]
    interface_ip: String,
}
#[derive(Args, Clone)]
struct DownArgs {
    #[arg(short, long)]
    peer_name: String,
    #[arg(short = 'i', long)]
    interface: String,
    #[arg(short = 'a', long)]
    interface_ip: String,
}

async fn add_link(handle: &Handle, name: &str) -> anyhow::Result<()> {
    let mut message = LinkMessage::default();

    // Set the interface name
    message
        .attributes
        .push(LinkAttribute::IfName(name.to_string()));

    // Set the link kind to wireguard
    message
        .attributes
        .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
            InfoKind::Wireguard,
        )]));

    println!("Adding link: {}", name);
    handle.link().add(message).execute().await?;
    Ok(())
}

async fn delete_link(handle: &Handle, name: &str) -> anyhow::Result<()> {
    // get links
    let mut link = handle.link().get().match_name(name).execute();
    if let Some(link) = link.try_next().await? {
        println!("Deleting link: {}", name);
        handle.link().del(link.header.index).execute().await?;
    } else {
        println!("No link found!")
    }
    Ok(())
}

async fn add_ip_to_interface(
    handle: &Handle,
    link_name: &str,
    ip: IpNetwork,
) -> anyhow::Result<()> {
    let mut links = handle.link().get().match_name(link_name).execute();
    if let Some(link) = links.try_next().await? {
        handle
            .address()
            .add(link.header.index, ip.ip(), ip.prefix())
            .execute()
            .await?
    }
    Ok(())
}

async fn set_link_state(handle: &Handle, name: &str, up: bool) -> anyhow::Result<()> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    if let Some(link) = links.try_next().await? {
        if up {
            handle
                .link()
                .set(LinkUnspec::new_with_index(link.header.index).up().build())
                .execute()
                .await?
        } else {
            handle
                .link()
                .set(LinkUnspec::new_with_index(link.header.index).down().build())
                .execute()
                .await?
        }
    } else {
        println!("no link link {name} found");
    }
    Ok(())
}

async fn set_mtu(handle: &Handle, name: &str, mtu: u32) -> anyhow::Result<()> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    if let Some(link) = links.try_next().await? {
        handle
            .link()
            .set(
                LinkUnspec::new_with_index(link.header.index)
                    .mtu(mtu)
                    .build(),
            )
            .execute()
            .await?
    } else {
        println!("no link link {name} found");
    }
    Ok(())
}

pub fn create_key_pair() -> anyhow::Result<()> {
    let key_pair = WireguardKeyPair::generate()?;

    let mut private_file = File::create("private.key")?;
    private_file.write_all(&key_pair.private_base64().into_bytes())?;

    let mut public_file = File::create("public.key")?;
    public_file.write_all(&key_pair.public_base64().into_bytes())?;

    Ok(())
}

async fn add_route(
    dest: &Ipv4Network,
    gateway: &Ipv4Network,
    index: u32,
    handle: &Handle,
) -> anyhow::Result<()> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(dest.ip(), dest.prefix())
        .gateway(gateway.ip())
        .output_interface(index)
        //.table_id(TEST_TABLE_ID)
        .build();
    handle.route().add(route).execute().await?;
    Ok(())
}

async fn del_route(
    dest: &Ipv4Network,
    gateway: &Ipv4Network,
    index: u32,
    handle: &Handle,
) -> anyhow::Result<()> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(dest.ip(), dest.prefix())
        .gateway(gateway.ip())
        .output_interface(index)
        //.table_id(TEST_TABLE_ID)
        .build();
    handle.route().del(route).execute().await?;
    Ok(())
}

async fn add_vpn_routes(handle: &Handle, interface: &String) -> anyhow::Result<()> {
    let index = get_interface_index(interface, handle).await?;

    let bottom_half = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(0, 0, 0, 0), 1)
        .output_interface(index)
        .build();

    handle.route().add(bottom_half).execute().await?;

    let top_half = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(128, 0, 0, 0), 1)
        .output_interface(index)
        .build();

    handle.route().add(top_half).execute().await?;

    Ok(())
}

async fn del_vpn_routes(handle: &Handle, interface: &String) -> anyhow::Result<()> {
    let index = get_interface_index(interface, handle).await?;

    let bottom_half = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(0, 0, 0, 0), 1)
        .output_interface(index)
        .build();

    handle.route().del(bottom_half).execute().await?;

    let top_half = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(128, 0, 0, 0), 1)
        .output_interface(index)
        .build();

    handle.route().del(top_half).execute().await?;

    Ok(())
}

async fn get_interface_index(name: &String, handle: &Handle) -> anyhow::Result<u32> {
    // get link by name
    let mut links = handle.link().get().match_name(name).execute();

    let link = links
        .try_next()
        .await?
        .ok_or_else(|| anyhow!("interface {name} not found!"))?;

    Ok(link.header.index)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // read config
    let config = wireguard_config::load_config(Path::new("./wgctl.toml")).unwrap();

    let (rtnetlink_connection, rtnetlink_handle, _) = rtnetlink_connection()?;
    let (genetlink_connection, mut genetlink_handle, _) = genetlink_connection()?;
    tokio::spawn(rtnetlink_connection);
    tokio::spawn(genetlink_connection);

    let mut wg_dev = WireGuardDevice::new(&config.interface, &config.peers, &mut genetlink_handle);
    match cli.command {
        Command::Create => {
            // if private key is missing stop
            if config.interface.private_key_path.is_empty() {
                return Err(anyhow!(
                    "private key path is empty in configuration, create private and public key with command: 'wgctl createkeys'"
                ));
            }

            let _res = add_link(&rtnetlink_handle, &config.interface.name.as_str()).await?;
            let ip = IpNetwork::from_str(config.interface.address.as_str())?;
            let _res =
                add_ip_to_interface(&rtnetlink_handle, &config.interface.name.as_str(), ip).await?;
            let _res =
                set_link_state(&rtnetlink_handle, config.interface.name.as_str(), true).await?;
            let _res = set_mtu(
                &rtnetlink_handle,
                config.interface.name.as_str(),
                config.interface.mtu.unwrap(),
            )
            .await?;

            wg_dev.create_from_config().await?;
        }
        Command::Up(args) => {
            let interface = args.interface;
            let interface_ip = args.interface_ip;
            let peer_name = args.peer_name;

            for peer in config.peers {
                if peer.peer_name == peer_name {
                    let endpoint: SocketAddrV4 =
                        peer.endpoint.as_ref().unwrap().as_str().parse()?;
                    let dest = Ipv4Network::new(endpoint.ip().clone(), 32)?;

                    // get interface network
                    let gateway: Ipv4Network = interface_ip.parse().unwrap_or_else(|_| {
                        eprintln!("invalid gateway");
                        std::process::exit(1);
                    });
                    let index = get_interface_index(&interface, &rtnetlink_handle).await?;
                    add_route(&dest, &gateway, index, &rtnetlink_handle).await?;
                }
            }
            // add default route splitting
            add_vpn_routes(&rtnetlink_handle, &config.interface.name).await?;
        }
        Command::Down(args) => {
            let interface = args.interface;
            let interface_ip = args.interface_ip;
            let peer_name = args.peer_name;

            for peer in config.peers {
                if peer.peer_name == peer_name {
                    let endpoint: SocketAddrV4 =
                        peer.endpoint.as_ref().unwrap().as_str().parse()?;
                    let dest = Ipv4Network::new(endpoint.ip().clone(), 32)?;

                    // get interface network
                    let gateway: Ipv4Network = interface_ip.parse().unwrap_or_else(|_| {
                        eprintln!("invalid gateway");
                        std::process::exit(1);
                    });
                    let index = get_interface_index(&interface, &rtnetlink_handle).await?;
                    del_route(&dest, &gateway, index, &rtnetlink_handle).await?;
                }
            }
            // add default route splitting
            del_vpn_routes(&rtnetlink_handle, &config.interface.name).await?;
        }
        Command::Delete => {
            let _res = delete_link(&rtnetlink_handle, &config.interface.name.as_str()).await?;
        }
        Command::CreateKeys => {
            let _res = create_key_pair()?;
            println!("Please include private and public keys in configuration!");
        }
        Command::Show => {
            let _res = wg_dev.show().await?;
        }
    }

    Ok(())
}
