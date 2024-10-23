use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU16,
    sync::atomic::{AtomicU16, Ordering},
    time::{Duration, Instant},
};

use tokio::sync::oneshot::Receiver;

use crate::{
    client::{AsyncSocket, Reply, ReplyMap},
    error::{Result, SurgeError},
    icmp::{icmpv4, icmpv6, IcmpPacket, PingIdentifier, PingSequence},
    is_linux_icmp_socket,
};

/// A Ping struct represents the state of one particular ping instance.
pub struct Pinger {
    pub host: IpAddr,
    pub ident: Option<PingIdentifier>,
    socket: AsyncSocket,
    reply_map: ReplyMap,
    last_sequence: AtomicU16,
}

impl Drop for Pinger {
    fn drop(&mut self) {
        if let Some(sequence) = NonZeroU16::new(self.last_sequence.load(Ordering::Relaxed)) {
            // Ensure no reply waiter is left hanging if this pinger is dropped while
            // waiting for a reply.
            self.reply_map
                .remove(self.host, self.ident, sequence.into());
        }
    }
}

impl Pinger {
    pub(crate) fn new(
        host: IpAddr,
        ident_hint: PingIdentifier,
        socket: AsyncSocket,
        response_map: ReplyMap,
    ) -> Pinger {
        let ident = if is_linux_icmp_socket!(socket.get_type()) {
            None
        } else {
            Some(ident_hint)
        };

        Pinger {
            host,
            ident,
            socket,
            reply_map: response_map,
            last_sequence: 0.into(),
        }
    }

    /// Send Ping request with sequence number.
    pub async fn ping(&self, seq: PingSequence, payload: &[u8]) -> Result<(IcmpPacket, Duration)> {
        let (send_time, reply_waiter) = self.ping_send(seq, payload).await?;
        self.ping_recv(send_time, reply_waiter).await
    }

    pub async fn ping_send(
        &self,
        seq: PingSequence,
        payload: &[u8],
    ) -> Result<(Instant, Receiver<Reply>)> {
        // Register to wait for a reply
        let reply_waiter = self.reply_map.new_waiter(self.host, self.ident, seq)?;

        // Send actual packet
        if let Err(e) = self.send_ping(seq, payload).await {
            self.reply_map.remove(self.host, self.ident, seq);
            return Err(e);
        }

        let send_time = Instant::now();
        self.last_sequence.store(seq.0.get(), Ordering::Relaxed);

        Ok((send_time, reply_waiter))
    }

    pub async fn ping_recv(
        &self,
        send_time: Instant,
        reply_waiter: Receiver<Reply>,
    ) -> Result<(IcmpPacket, Duration)> {
        let reply = reply_waiter.await.map_err(|_| SurgeError::NetworkError)?;
        let duration = reply.timestamp.saturating_duration_since(send_time);
        Ok((reply.packet, duration))
    }

    /// Send a ping packet (useful, when you don't need a reply).
    pub async fn send_ping(&self, seq: PingSequence, payload: &[u8]) -> Result<()> {
        // Create and send ping packet.
        let mut packet = match self.host {
            IpAddr::V4(_) => icmpv4::make_icmpv4_echo_packet(
                self.ident.unwrap_or(PingIdentifier(0)),
                seq,
                self.socket.get_type(),
                payload,
            )?,
            IpAddr::V6(_) => icmpv6::make_icmpv6_echo_packet(
                self.ident.unwrap_or(PingIdentifier(0)),
                seq,
                payload,
            )?,
        };

        self.socket
            .send_to(&mut packet, &SocketAddr::new(self.host, 0))
            .await?;

        Ok(())
    }
}
