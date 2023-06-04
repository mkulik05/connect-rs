use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{thread, time};
use std::time::Duration;
use crate::thread::sleep;

const MAX_LEN: usize = 25;
const ADDR: &str = "0.0.0.0:12444";
const STUN_ADDR: &str = "20.82.177.124:34343";

fn main() -> std::io::Result<()> {
    let socket;
    match UdpSocket::bind(ADDR) {
        Ok(s) => socket = s,
        Err(e) => panic!("{}", e),
    }

    let mapped_address = loop {
        if let Ok(addr) = get_mapped_addr(&socket, STUN_ADDR) {
            break addr;
        }
    };
    println!("Mapped address: {}", mapped_address);
    let peer_address = get_remote_address().unwrap();
    println!("Peer address: {}", peer_address);

    println!("Starting punching...");
    punch(
        socket.try_clone().expect("faiiled cloning socker"),
        &peer_address,
    ).unwrap();
    println!("Finished???");
    test_connection(socket, &peer_address);

    println!("{}", mapped_address);
    Ok(())
}

fn test_connection(socket: UdpSocket, addr: &String) {
    let socket2 = socket.try_clone().expect("Can't clone udp socket");
    let buf = [0; 1];
    let str_addr = (*addr).clone();
    thread::spawn(move || {
        let mut buf = [0; 256];
        loop {
            let (amnt, src) = socket2.recv_from(&mut buf).unwrap();
            if src.to_string() == str_addr {
                let data_bytes = &buf[..amnt];
                println!("{}", std::str::from_utf8(data_bytes).expect("Bad msg"));
            }
        }
    });
    loop {
        println!("> ");
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf).unwrap();
        socket.send_to(&buf.into_bytes(), addr).unwrap();
        println!("Sent");
    }
}

fn punch(socket: UdpSocket, addr: &String) -> std::io::Result<()> {
    let socket2 = socket.try_clone().expect("Can't clone udp socket");
    let was_punched = Arc::new(Mutex::new(AtomicBool::new(false)));
    let mut buf = [0; 1];
    let str_addr = (*addr).clone();
    let check_inp_udp = {
        let was_punched = was_punched.clone();
        thread::spawn(move || {
            let mut got_packets = 0;
            loop {
                let (_, src) = socket2.recv_from(&mut buf).unwrap();
                if src.to_string() == str_addr {
                    got_packets += 1
                }
                if got_packets >= 10 {
                    let guard = was_punched.lock().unwrap();
                    guard.store(true, Ordering::SeqCst);
                    break;
                }
            }
        })
    };
    loop {
        socket.send_to(&buf, addr).unwrap();
        sleep(Duration::from_millis(200));
        let guard = was_punched.lock().unwrap();
        if guard.load(Ordering::SeqCst) {
            break;
        }
    }
    check_inp_udp.join().unwrap();
    Ok(())
}

fn get_remote_address() -> std::io::Result<String> {
    println!("Input remote addr:");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn get_mapped_addr(socket: &UdpSocket, stun_addr: &str) -> std::io::Result<String> {
    let buf = [0; 1];
    socket.send_to(&buf, stun_addr)?;
    let mut buf = [0; MAX_LEN];
    let mapped_address = loop {
        let (amnt, src) = socket.recv_from(&mut buf)?;
        if src.to_string() == stun_addr {
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
    Ok(mapped_address.to_string())
}
