use std::error::Error;
use std::iter::FromIterator;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use rand::random;
use surge_ping::{Client, Config, IcmpPacket, PingIdentifier, PingSequence, ICMP};
use tokio::sync::mpsc;
use tokio::time;

#[derive(Default, Debug)]
struct Answer {
    host: String,
    transmitted: usize,
    received: usize,
    durations: Vec<Duration>,
}

impl Answer {
    fn new(host: String, transmitted: usize) -> Answer {
        Answer {
            host,
            transmitted,
            received: 0,
            durations: Vec::new(),
        }
    }

    fn update(&mut self, dur: Option<Duration>) {
        if let Some(dur) = dur {
            self.received += 1;
            self.durations.push(dur);
        }
    }

    fn min(&self) -> f64 {
        self.durations
            .iter()
            .min()
            .map(|dur| dur.as_secs_f64() * 1000.0)
            .unwrap_or_default()
    }

    fn max(&self) -> f64 {
        self.durations
            .iter()
            .max()
            .map(|dur| dur.as_secs_f64() * 1000.0)
            .unwrap_or_default()
    }

    fn avg(&self) -> f64 {
        let sum: Duration = self.durations.iter().sum();
        sum.checked_div(self.durations.len() as u32)
            .map(|dur| dur.as_secs_f64() * 1000.0)
            .unwrap_or_default()
    }

    fn stddev(&self) -> f64 {
        let sum = self
            .durations
            .iter()
            .map(|dur| dur.as_secs_f64().powi(2) * 1e6)
            .sum::<f64>();
        (sum / self.durations.len() as f64 - self.avg().powi(2)).sqrt()
    }

    fn output(&self) {
        println!("\n--- {} ping statistics ---", self.host);
        println!(
            "{} packets transmitted, {} packets received, {:.2}% packet loss",
            self.transmitted,
            self.received,
            100.0 * (1.0 - (self.received as f64 / self.transmitted as f64)),
        );
        if self.received > 1 {
            let (min, avg, max, stddev) = (self.min(), self.avg(), self.max(), self.stddev());
            println!("round-trip min/avg/max/stddev = {min:.3}/{avg:.3}/{max:.3}/{stddev:.3} ms");
        }
    }
}

#[derive(Parser, Debug)]
#[clap(name = "surge-ping")]
struct Args {
    /// Destination host or address
    host: String,

    /// Use IPv4
    #[clap(short = '4', group("ip version"))]
    v4: bool,

    /// Use IPv6
    #[clap(short = '6', group("ip version"))]
    v6: bool,

    /// Wait time in seconds between sending each packet
    #[clap(short = 'i', long, default_value = "1.0")]
    interval: f64,

    /// Specify the number of data bytes to be sent
    #[clap(short = 's', long, default_value = "56")]
    size: usize,

    /// Stop after sending <count> ECHO_REQUEST packets
    #[clap(short = 'c', long, default_value = "5")]
    count: usize,

    /// Source packets with the given interface ip address
    #[clap(short = 'I', long)]
    interface: Option<IpAddr>,

    /// Specify a timeout in seconds, beginning once the last ping is sent
    #[clap(short = 'w', long, default_value = "1.0")]
    wait_timeout: f64,
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn Error>> {
    let Args {
        host,
        v4,
        v6,
        interval,
        size,
        count,
        interface,
        wait_timeout,
    } = Args::parse();

    let is_ipv6 = v6 || (!v4 && matches!(interface, Some(IpAddr::V6(_))));

    if is_ipv6 && matches!(interface, Some(IpAddr::V4(_))) {
        eprintln!("Fatal error: interface is IPv4 but ping is IPv6.");
        return Ok(ExitCode::FAILURE);
    }

    let ip = tokio::net::lookup_host(format!("{host}:0"))
        .await
        .map_err(|err| format!("host lookup error: {err}"))?
        .map(|s| s.ip())
        .find(|ip| ip.is_ipv6() == is_ipv6)
        .ok_or("host lookup error")?;

    let mut builder = Config::builder();
    if let Some(ip) = interface {
        builder = builder.bind(SocketAddr::new(ip, 0));
    }
    if is_ipv6 {
        builder = builder.kind(ICMP::V6);
    }
    let client = Client::new(&builder.build()).unwrap();

    println!("PING {host} ({ip}): {size} data bytes");

    let mut global_timeout = Box::pin(time::sleep(Duration::MAX));
    let pinger = Arc::new(client.pinger(ip, PingIdentifier(random())).await);
    let (tx, mut rx) = mpsc::unbounded_channel();

    tokio::spawn({
        let pinger = pinger.clone();
        let payload = (b'A'..=b'Z').cycle().take(size).collect::<Vec<_>>();
        let mut interval = time::interval(Duration::from_millis((interval * 1000.0) as u64));
        async move {
            let count = count as u16;
            for idx in 0..count {
                interval.tick().await;
                let idx = NonZeroU16::new(idx + 1).unwrap();
                let last = idx.get() == count;
                let send_data = pinger.ping_send(PingSequence(idx), &payload).await.unwrap();
                tx.send((send_data, last)).unwrap();
            }
        }
    });

    let mut answer = Answer::new(host, count);
    let mut fuo = FuturesUnordered::from_iter([pending().boxed()]);
    let mut remaining = count;
    let mut success = true;

    loop {
        tokio::select! {
            _ = &mut global_timeout => {
                success = false;
                println!("Timeout triggered when waiting for replies.");
                break;
            }
            Some(((send_time, reply_waiter), last)) = rx.recv() => {
                fuo.push(Box::pin(pinger.ping_recv(send_time, reply_waiter)));
                if last {
                    global_timeout = Box::pin(time::sleep(Duration::from_millis((wait_timeout * 1000.0) as u64)));
                }
            }
            Some(res) = fuo.next() => {
                match res {
                    Ok((IcmpPacket::V4(reply), dur)) => {
                        println!(
                            "{} bytes from {}: icmp_seq={} time={dur:0.3?}",
                            reply.get_size(),
                            reply.get_source(),
                            reply.get_sequence(),
                        );
                        answer.update(Some(dur));
                    }
                    Ok((IcmpPacket::V6(reply), dur)) => {
                        println!(
                            "{} bytes from {}: icmp_seq={} time={dur:0.3?}",
                            reply.get_size(),
                            reply.get_source(),
                            reply.get_sequence(),
                        );
                        answer.update(Some(dur));
                    }
                    Err(err) => {
                        success = false;
                        println!("{err}");
                        answer.update(None);
                    }
                }
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
    }

    answer.output();

    if success {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}
