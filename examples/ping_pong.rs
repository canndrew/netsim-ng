use {
    std::{
        net::SocketAddr,
        str,
    },
    tokio::net::UdpSocket,
    netsim_ng::Machine,
    net_literals::ipv4,
    futures::join,
};


// This example creates two network-isolated threads and gives each a network interface with which
// they can send messages back-and-forth.
#[tokio::main]
async fn main() {
    let ipv4_addr_0 = ipv4!("10.1.2.3");
    let port_0 = 45666;
    let addr_0 = SocketAddr::from((ipv4_addr_0, port_0));

    let ipv4_addr_1 = ipv4!("192.168.5.5");
    let port_1 = 5555;
    let addr_1 = SocketAddr::from((ipv4_addr_1, port_1));

    // Create two machines. A machine initially has no network interfaces and has its own tokio
    // runtime on which we can spawn tasks.
    let machine_0 = Machine::new().unwrap();
    let machine_1 = Machine::new().unwrap();

    // Give each machine a network interface.
    let iface_0 = {
        machine_0
        .add_ip_iface()
        .ipv4_addr(ipv4_addr_0)
        .await
        .unwrap()
    };
    let iface_1 = {
        machine_1
        .add_ip_iface()
        .ipv4_addr(ipv4_addr_1)
        .await
        .unwrap()
    };

    // Connect the network interfaces directly to each other.
    netsim_ng::connect(iface_0, iface_1);

    // Machine 0 receives a UDP packet then replies with a UDP packet.
    let join_handle_0 = machine_0.spawn(async move {
        let socket = UdpSocket::bind(addr_0).await.unwrap();

        let mut recv_bytes = [0u8; 100];
        let (recv_len, peer_addr) = socket.recv_from(&mut recv_bytes).await.unwrap();
        assert_eq!(peer_addr, addr_1);
        let recv_msg = str::from_utf8(&recv_bytes[..recv_len]).unwrap();
        println!("received msg: '{recv_msg}'");

        let send_msg = "pong";
        let send_len = socket.send_to(send_msg.as_bytes(), addr_1).await.unwrap();
        assert_eq!(send_len, send_msg.len());
        println!("sent reply: '{send_msg}'");
    });

    // Machine 1 sends a UDP packet then waits for the reply.
    let join_handle_1 = machine_1.spawn(async move {
        let socket = UdpSocket::bind(addr_1).await.unwrap();

        let send_msg = "ping";
        let send_len = socket.send_to(send_msg.as_bytes(), addr_0).await.unwrap();
        assert_eq!(send_len, send_msg.len());
        println!("sent msg: '{send_msg}'");

        let mut recv_bytes = [0u8; 100];
        let (recv_len, peer_addr) = socket.recv_from(&mut recv_bytes).await.unwrap();
        assert_eq!(peer_addr, addr_0);
        let recv_msg = str::from_utf8(&recv_bytes[..recv_len]).unwrap();
        println!("received reply: '{recv_msg}'");
    });

    // Wait for both machines to run their tasks to completion.
    let (task_result_0, task_result_1) = join!(join_handle_0.join(), join_handle_1.join());
    let () = task_result_0.unwrap().unwrap();
    let () = task_result_1.unwrap().unwrap();
}
