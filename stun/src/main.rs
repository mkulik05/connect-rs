use std::net::UdpSocket;

const MAX_LEN: usize = 25;
const ADDR: &str = "0.0.0.0:34343";

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind(ADDR)?;
    let mut buf = [0; 1];
    loop {
        let (_, src) = socket.recv_from(&mut buf)?;
        let bytes_addr = src.to_string().into_bytes();
        if bytes_addr.len() > MAX_LEN {
            eprintln!("Too long address: {}", src.to_string());
        } else {
            socket.send_to(&bytes_addr[..], &src)?;
        } 
    }
}