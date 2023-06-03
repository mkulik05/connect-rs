use std::net::UdpSocket;

const MAX_LEN: usize = 25;
const ADDR: &str = "0.0.0.0:12444";
const REMOTE_ADDR: &str = "20.82.177.124:34343";

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind(ADDR)?;
    let buf = [0; 1];
    socket.send_to(&buf,  REMOTE_ADDR)?;
    let mut buf = [0; MAX_LEN];
    let mapped_address = loop {
        let (amnt, src) = socket.recv_from(&mut buf)?;
        if src.to_string() == REMOTE_ADDR {
            let buf = &buf[..amnt];
            let addr = std::str::from_utf8(buf);
            match addr {
                Ok(value) => break value,
                Err(_) => {
                    eprintln!("error during result decoding. Bytes: {:?}", buf)
                }
            }
        };
    };
    println!("{mapped_address}");
    Ok(())
}