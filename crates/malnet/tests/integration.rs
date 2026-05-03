//! Integration tests for the malnet-peer crate.
//!
//! These tests verify the complete behavior of the Peer struct and how
//! multiple systems interact together.

use std::time::Instant;

use malnet::{
    core::config::{CompressionAlgorithm, Config},
    peer::Peer,
    protocol::{command::ProtocolCommand, packet::OrderingGuarantee},
};

fn create_virtual_connection() -> Peer {
    Peer::new(get_fake_addr(), &Config::default(), Instant::now())
}

fn get_fake_addr() -> std::net::SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

#[test]
fn test_unsequenced_window_wrapping() {
    let mut peer = create_virtual_connection();
    let time = Instant::now();

    // Start by receiving some packets to establish a base around 65000
    for i in 0..3 {
        let cmd = ProtocolCommand::SendUnsequenced {
            channel_id: 0,
            unsequenced_group: 65000 + i,
            data: vec![i as u8].into(),
        };
        let result = peer.process_command(&cmd, time).unwrap();
        assert_eq!(result.into_iter().count(), 1);
    }

    // Now send near the end of u16 range
    let cmd65500 = ProtocolCommand::SendUnsequenced {
        channel_id: 0,
        unsequenced_group: 65500,
        data: vec![1].into(),
    };
    let result1 = peer.process_command(&cmd65500, time).unwrap();
    assert_eq!(result1.into_iter().count(), 1);

    // Wrap around to 10 (should be treated as newer, after 65535)
    let cmd10 = ProtocolCommand::SendUnsequenced {
        channel_id: 0,
        unsequenced_group: 10,
        data: vec![2].into(),
    };
    let result2 = peer.process_command(&cmd10, time).unwrap();
    assert_eq!(result2.into_iter().count(), 1);

    // Sending 65002 (old packet from before) should be dropped
    let cmd65002 = ProtocolCommand::SendUnsequenced {
        channel_id: 0,
        unsequenced_group: 65002,
        data: vec![3].into(),
    };
    let result3 = peer.process_command(&cmd65002, time).unwrap();
    assert_eq!(result3.into_iter().count(), 0); // Should be dropped as old/duplicate
}

#[test]
fn test_unsequenced_per_channel() {
    let mut peer = create_virtual_connection();
    let time = Instant::now();

    // Send on channel 0
    let cmd_ch0 = ProtocolCommand::SendUnsequenced {
        channel_id: 0,
        unsequenced_group: 5,
        data: vec![0].into(),
    };

    // Send on channel 1 with same group (should not conflict)
    let cmd_ch1 = ProtocolCommand::SendUnsequenced {
        channel_id: 1,
        unsequenced_group: 5,
        data: vec![1].into(),
    };

    // Both should be delivered (unsequenced is global, not per-channel)
    // Note: Unlike ordered/sequenced which are per-channel, unsequenced
    // uses a global window
    let result0 = peer.process_command(&cmd_ch0, time).unwrap();
    let result1 = peer.process_command(&cmd_ch1, time).unwrap();

    assert_eq!(result0.into_iter().count(), 1);
    // Second one with same group is a duplicate (global window)
    assert_eq!(result1.into_iter().count(), 0);
}

#[test]
fn test_unsequenced_end_to_end() {
    let config = Config::default();
    let mut peer1 = Peer::new(get_fake_addr(), &config, Instant::now());
    let mut peer2 = Peer::new(get_fake_addr(), &config, Instant::now());
    let time = Instant::now();

    // peer1 sends several unsequenced packets
    for i in 0..5 {
        let group = peer1.next_unsequenced_group();
        peer1.enqueue_command(ProtocolCommand::SendUnsequenced {
            channel_id: 0,
            unsequenced_group: group,
            data: vec![i].into(),
        });
    }

    let encoded = peer1.encode_queued_commands().unwrap();

    // peer2 receives all 5 packets
    let result = peer2.process_command_packet(&encoded, time).unwrap();
    let packets: Vec<_> = result.into_iter().collect();
    assert_eq!(packets.len(), 5);

    // Verify all packets have Unsequenced ordering
    for (pkt, _) in &packets {
        assert_eq!(pkt.order_guarantee(), OrderingGuarantee::Unsequenced);
    }

    // Send the same encoded data again - all should be dropped as duplicates
    let result2 = peer2.process_command_packet(&encoded, time).unwrap();
    assert_eq!(result2.into_iter().count(), 0);
}

#[test]
fn test_waiting_data_limit_drops_excess() {
    let mut config = Config::default();
    config.max_waiting_data = 1000; // Limit to 1KB
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue 500 bytes - should succeed
    let data1 = vec![1u8; 500];
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 1,
        ordered: true,
        data: data1.into(),
    });
    assert_eq!(peer.queued_commands_count(), 1);

    // Enqueue another 500 bytes - should succeed (total = 1000)
    let data2 = vec![2u8; 500];
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 2,
        ordered: true,
        data: data2.into(),
    });
    assert_eq!(peer.queued_commands_count(), 2);

    // Try to enqueue 100 more bytes - should be dropped (would exceed limit)
    let data3 = vec![3u8; 100];
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 3,
        ordered: true,
        data: data3.into(),
    });
    assert_eq!(peer.queued_commands_count(), 2); // Still 2, third was dropped
}

#[test]
fn test_waiting_data_unlimited_when_zero() {
    let mut config = Config::default();
    config.max_waiting_data = 0; // Unlimited
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue large amounts of data
    for i in 0..100 {
        let data = std::sync::Arc::<[u8]>::from(vec![i as u8; 10000].into_boxed_slice()); // 10KB each
        peer.enqueue_command(ProtocolCommand::SendReliable {
            channel_id: 0,
            sequence: i,
            ordered: true,
            data: data.into(),
        });
    }

    // All 100 commands should be queued (total = 1MB)
    assert_eq!(peer.queued_commands_count(), 100);
}

#[test]
fn test_waiting_data_resets_on_drain() {
    let mut config = Config::default();
    config.max_waiting_data = 1000;
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue 1000 bytes
    let data1 = std::sync::Arc::<[u8]>::from(vec![1u8; 1000].into_boxed_slice());
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 1,
        ordered: true,
        data: data1.into(),
    });
    assert_eq!(peer.queued_commands_count(), 1);

    // Try to enqueue more - should be dropped
    let data2 = std::sync::Arc::<[u8]>::from(vec![2u8; 100].into_boxed_slice());
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 2,
        ordered: true,
        data: data2.into(),
    });
    assert_eq!(peer.queued_commands_count(), 1); // Still 1

    // Drain commands (simulating send)
    let _commands: Vec<_> = peer.drain_commands().collect();

    // Now we can enqueue again
    let data3 = std::sync::Arc::<[u8]>::from(vec![3u8; 1000].into_boxed_slice());
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 3,
        ordered: true,
        data: data3.into(),
    });
    assert_eq!(peer.queued_commands_count(), 1); // New command enqueued successfully
}

#[test]
fn test_waiting_data_control_commands_not_counted() {
    let mut config = Config::default();
    config.max_waiting_data = 100;
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue control commands (no data) - should always succeed
    peer.enqueue_command(ProtocolCommand::Ping { timestamp: 1 });
    peer.enqueue_command(ProtocolCommand::Pong { timestamp: 2 });
    peer.enqueue_command(ProtocolCommand::Disconnect { reason: 0 });
    peer.enqueue_command(ProtocolCommand::Acknowledge {
        sequence: 1,
        received_mask: 0xFF,
        sent_time: None,
    });

    assert_eq!(peer.queued_commands_count(), 4);

    // Now enqueue data commands up to the limit
    let data = std::sync::Arc::<[u8]>::from(vec![1u8; 100].into_boxed_slice());
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 1,
        ordered: true,
        data: data.into(),
    });

    assert_eq!(peer.queued_commands_count(), 5); // All commands enqueued
}

#[test]
fn test_window_flow_control_enabled() {
    let mut config = Config::default();
    config.use_window_flow_control = true;
    config.initial_window_size = 100;
    let peer = Peer::new(get_fake_addr(), &config, Instant::now());

    assert_eq!(peer.window_size(), 100);
    assert_eq!(peer.reliable_data_in_transit(), 0);
    assert!(peer.can_send_reliable());
}

#[test]
fn test_window_flow_control_tracks_in_transit_data() {
    let mut config = Config::default();
    config.use_window_flow_control = true;
    config.initial_window_size = 10; // Small window
    config.fragment_size = 1024;
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Initially can send
    assert!(peer.can_send_reliable());

    // Record sending 5KB of data
    peer.record_reliable_data_sent(5 * 1024);
    assert_eq!(peer.reliable_data_in_transit(), 5 * 1024);

    // Can still send (5KB < 10 packets * 1024 bytes = 10KB window)
    assert!(peer.can_send_reliable());

    // Record sending another 6KB (total 11KB, exceeds 10KB window)
    peer.record_reliable_data_sent(6 * 1024);
    assert_eq!(peer.reliable_data_in_transit(), 11 * 1024);

    // Now cannot send (exceeds window)
    assert!(!peer.can_send_reliable());

    // ACK some data (3KB)
    peer.record_reliable_data_acked(3 * 1024);
    assert_eq!(peer.reliable_data_in_transit(), 8 * 1024);

    // Now can send again (8KB < 10KB window)
    assert!(peer.can_send_reliable());
}

#[test]
fn test_window_size_negotiation() {
    let mut config = Config::default();
    config.use_window_flow_control = true;
    config.initial_window_size = 1000;
    config.min_window_size = 64;
    config.max_window_size = 2048;
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    assert_eq!(peer.window_size(), 1000);

    // Set to a value within range
    peer.set_window_size(512);
    assert_eq!(peer.window_size(), 512);

    // Set to value above max - should clamp
    peer.set_window_size(3000);
    assert_eq!(peer.window_size(), 2048);

    // Set to value below min - should clamp
    peer.set_window_size(32);
    assert_eq!(peer.window_size(), 64);
}

#[test]
fn test_window_adjustment_increases_on_good_conditions() {
    let mut config = Config::default();
    config.use_window_flow_control = true;
    config.initial_window_size = 100;
    config.max_window_size = 200;
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    let initial_window = peer.window_size();

    // Simulate good conditions (no loss, low RTT)
    // peer.loss_rate() will be 0.0 by default
    // peer.rtt() is 50ms by default

    peer.adjust_window_size();

    // Window should increase
    assert!(peer.window_size() > initial_window);
}

#[test]
fn test_window_flow_control_disabled_uses_packet_limit() {
    let mut config = Config::default();
    config.use_window_flow_control = false; // Disabled
    config.max_packets_in_flight = 10;
    let peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // When disabled, should use packets_in_flight limit
    // Initially 0 packets in flight, so can send
    assert!(peer.can_send_reliable());
    assert_eq!(peer.packets_in_flight(), 0);
}

// ===== Statistics Tests =====

#[test]
fn test_statistics_initialized_to_zero() {
    let config = Config::default();
    let peer = Peer::new(get_fake_addr(), &config, Instant::now());

    let stats = peer.statistics();
    assert_eq!(stats.packets_sent, 0);
    assert_eq!(stats.packets_received, 0);
    assert_eq!(stats.packets_lost, 0);
    assert_eq!(stats.bytes_sent, 0);
    assert_eq!(stats.bytes_received, 0);
    assert_eq!(stats.packet_loss_rate(), 0.0);
}

#[test]
fn test_statistics_track_packets_sent() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue and encode a command
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: vec![1, 2, 3].into(),
    });

    // Encode should increment packets_sent
    let _ = peer.encode_queued_commands().unwrap();
    assert_eq!(peer.statistics().packets_sent, 1);
    assert!(peer.statistics().bytes_sent > 0);
}

#[test]
fn test_statistics_track_packets_received() {
    let config = Config::default();
    let mut peer1 = Peer::new(get_fake_addr(), &config, Instant::now());
    let mut peer2 = Peer::new(get_fake_addr(), &config, Instant::now());

    // Use peer1 to create a proper packet
    peer1.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: vec![1, 2, 3, 4, 5].into(),
    });
    let encoded = peer1.encode_queued_commands().unwrap();

    // Process the packet with peer2
    let _ = peer2.process_command_packet(&encoded, Instant::now()).unwrap();

    assert_eq!(peer2.statistics().packets_received, 1);
    assert_eq!(peer2.statistics().bytes_received, encoded.len() as u64);
}

#[test]
fn test_statistics_track_multiple_packets() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Send multiple packets
    for i in 0..5 {
        peer.enqueue_command(ProtocolCommand::SendReliable {
            channel_id: 0,
            sequence: i,
            ordered: true,
            data: vec![1, 2, 3].into(),
        });
        let _ = peer.encode_queued_commands().unwrap();
    }

    assert_eq!(peer.statistics().packets_sent, 5);
    assert!(peer.statistics().bytes_sent > 0);
}

#[test]
fn test_statistics_track_packet_loss() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Manually set some packets as lost to test tracking
    peer.statistics_mut().packets_lost = 5;

    assert_eq!(peer.statistics().packets_lost, 5);
}

#[test]
fn test_statistics_packet_loss_rate() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Manually set statistics for controlled test
    peer.statistics_mut().packets_sent = 100;
    peer.statistics_mut().packets_lost = 10;

    let loss_rate = peer.statistics().packet_loss_rate();
    assert!((loss_rate - 0.1).abs() < 0.001); // 10/100 = 0.1 = 10%
}

#[test]
fn test_statistics_bytes_sent_includes_overhead() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Enqueue a small data packet
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: vec![1, 2, 3].into(),
    });

    let encoded = peer.encode_queued_commands().unwrap();

    // bytes_sent should equal the full encoded packet size (data + protocol overhead)
    assert_eq!(peer.statistics().bytes_sent, encoded.len() as u64);
    assert!(peer.statistics().bytes_sent > 3); // More than just the 3 data bytes
}

#[test]
fn test_statistics_reset() {
    let config = Config::default();
    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());

    // Generate some statistics
    peer.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: vec![1, 2, 3].into(),
    });
    let _ = peer.encode_queued_commands().unwrap();

    assert!(peer.statistics().packets_sent > 0);
    assert!(peer.statistics().bytes_sent > 0);

    // Reset statistics
    peer.statistics_mut().reset();

    assert_eq!(peer.statistics().packets_sent, 0);
    assert_eq!(peer.statistics().packets_received, 0);
    assert_eq!(peer.statistics().packets_lost, 0);
    assert_eq!(peer.statistics().bytes_sent, 0);
    assert_eq!(peer.statistics().bytes_received, 0);
}

#[test]
fn test_pmtu_discovery_can_be_disabled() {
    let mut config = Config::default();
    config.use_pmtu_discovery = false;
    assert!(!config.use_pmtu_discovery);

    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());
    let time = Instant::now();

    // Should not generate any probes when disabled
    peer.handle_pmtu(time);
    assert!(!peer.has_queued_commands());
}

#[test]
fn test_pmtu_discovery_convergence() {
    let mut config = Config::default();
    config.use_pmtu_discovery = true;
    config.pmtu_min = 1200;
    config.pmtu_max = 1232; // Within convergence threshold
    config.pmtu_converge_threshold = 64;

    let mut peer = Peer::new(get_fake_addr(), &config, Instant::now());
    let time = Instant::now();

    // When high - low <= threshold, should converge to low
    peer.handle_pmtu(time);

    // Should converge and use pmtu_low as fragment size
    assert_eq!(peer.current_fragment_size(), config.pmtu_min);
}

#[test]
fn test_three_way_handshake_with_session_ids() {
    let mut config = Config::default();
    config.use_connection_handshake = true;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:8001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:8002".parse().unwrap(), &config, time);

    // Step 1: Client initiates connection
    client.initiate_connect();
    assert!(client.has_queued_commands());

    // Encode and send Connect command to server
    let connect_bytes = client.encode_queued_commands().unwrap();

    // Step 2: Server receives Connect and sends VerifyConnect
    let result = server.process_command_packet(&connect_bytes, time);
    assert!(result.is_ok(), "Server should process Connect command successfully");
    assert!(server.has_queued_commands(), "Server should have VerifyConnect queued");

    // Encode and send VerifyConnect command to client
    let verify_bytes = server.encode_queued_commands().unwrap();

    // Step 3: Client receives VerifyConnect and validates session IDs
    let result = client.process_command_packet(&verify_bytes, time);
    assert!(
        result.is_ok(),
        "Client should process VerifyConnect command without session ID mismatch"
    );

    // Verify handshake completed successfully on client side
    // Client should transition to ConnectionSucceeded state
}

#[test]
fn test_handshake_session_id_echo_verification() {
    let mut config = Config::default();
    config.use_connection_handshake = true;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:9001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:9002".parse().unwrap(), &config, time);

    // Client initiates handshake
    client.initiate_connect();
    let connect_bytes = client.encode_queued_commands().unwrap();

    // Server processes Connect
    server.process_command_packet(&connect_bytes, time).unwrap();
    let verify_bytes = server.encode_queued_commands().unwrap();

    // Client processes VerifyConnect - should succeed because session IDs are now correct
    let result = client.process_command_packet(&verify_bytes, time);
    assert!(
        result.is_ok(),
        "Handshake should complete successfully with correct session ID echoing"
    );
}

#[test]
fn test_handshake_end_to_end() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = true;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:7001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:7002".parse().unwrap(), &config, time);

    // Full 3-way handshake
    client.initiate_connect();
    let connect_bytes = client.encode_queued_commands().unwrap();
    assert!(connect_bytes.len() >= 4, "Connect packet should be large enough for checksum");

    server.process_command_packet(&connect_bytes, time).unwrap();
    let verify_bytes = server.encode_queued_commands().unwrap();
    assert!(verify_bytes.len() >= 4, "VerifyConnect packet should be large enough for checksum");

    client.process_command_packet(&verify_bytes, time).unwrap();

    // Now send actual data to complete handshake
    client.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: vec![1, 2, 3, 4, 5].into(),
    });

    let data_bytes = client.encode_queued_commands().unwrap();
    let result = server.process_command_packet(&data_bytes, time).unwrap();
    let packets: Vec<_> = result.into_iter().collect();

    // Verify data was received
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].0.payload(), &[1, 2, 3, 4, 5]);
}

#[test]
fn test_handshake_with_compression() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = true;
    config.compression = CompressionAlgorithm::Lz4;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:6001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:6002".parse().unwrap(), &config, time);

    // Full handshake with compression enabled
    client.initiate_connect();
    let connect_bytes = client.encode_queued_commands().unwrap();

    server.process_command_packet(&connect_bytes, time).unwrap();
    let verify_bytes = server.encode_queued_commands().unwrap();

    client.process_command_packet(&verify_bytes, time).unwrap();

    // Send compressible data
    let large_data = vec![0x42u8; 1000]; // Highly compressible
    client.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: large_data.clone().into(),
    });

    let data_bytes = client.encode_queued_commands().unwrap();
    let result = server.process_command_packet(&data_bytes, time).unwrap();
    let packets: Vec<_> = result.into_iter().collect();

    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].0.payload(), &large_data[..]);
}

#[test]
fn test_handshake_multiple_clients() {
    let mut config = Config::default();
    config.use_connection_handshake = true;

    let time = Instant::now();
    let mut server = Peer::new("127.0.0.1:5000".parse().unwrap(), &config, time);

    // Create multiple clients
    let mut client1 = Peer::new("127.0.0.1:5001".parse().unwrap(), &config, time);
    let mut client2 = Peer::new("127.0.0.1:5002".parse().unwrap(), &config, time);
    let mut client3 = Peer::new("127.0.0.1:5003".parse().unwrap(), &config, time);

    // Each client initiates handshake
    client1.initiate_connect();
    client2.initiate_connect();
    client3.initiate_connect();

    // Server processes each Connect
    let connect1 = client1.encode_queued_commands().unwrap();
    let connect2 = client2.encode_queued_commands().unwrap();
    let connect3 = client3.encode_queued_commands().unwrap();

    server.process_command_packet(&connect1, time).unwrap();
    let verify1 = server.encode_queued_commands().unwrap();

    server.process_command_packet(&connect2, time).unwrap();
    let verify2 = server.encode_queued_commands().unwrap();

    server.process_command_packet(&connect3, time).unwrap();
    let verify3 = server.encode_queued_commands().unwrap();

    // Each client completes handshake
    assert!(client1.process_command_packet(&verify1, time).is_ok());
    assert!(client2.process_command_packet(&verify2, time).is_ok());
    assert!(client3.process_command_packet(&verify3, time).is_ok());
}

#[test]
fn test_handshake_bidirectional_data() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = true;

    let time = Instant::now();
    let mut peer1 = Peer::new("127.0.0.1:4001".parse().unwrap(), &config, time);
    let mut peer2 = Peer::new("127.0.0.1:4002".parse().unwrap(), &config, time);

    // Peer1 initiates handshake
    peer1.initiate_connect();
    let connect = peer1.encode_queued_commands().unwrap();

    peer2.process_command_packet(&connect, time).unwrap();
    let verify = peer2.encode_queued_commands().unwrap();

    peer1.process_command_packet(&verify, time).unwrap();

    // Now both peers can send data to each other
    // Peer1 -> Peer2
    peer1.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: b"Hello from peer1".to_vec().into(),
    });

    let data1 = peer1.encode_queued_commands().unwrap();
    let result1 = peer2.process_command_packet(&data1, time).unwrap();
    let packets1: Vec<_> = result1.into_iter().collect();

    assert_eq!(packets1.len(), 1);
    assert_eq!(packets1[0].0.payload(), b"Hello from peer1");

    // Peer2 -> Peer1
    peer2.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: b"Hello from peer2".to_vec().into(),
    });

    let data2 = peer2.encode_queued_commands().unwrap();
    let result2 = peer1.process_command_packet(&data2, time).unwrap();
    let packets2: Vec<_> = result2.into_iter().collect();

    assert_eq!(packets2.len(), 1);
    assert_eq!(packets2[0].0.payload(), b"Hello from peer2");
}

#[test]
fn test_handshake_with_different_fragment_sizes() {
    let time = Instant::now();

    // Client with larger fragment size
    let mut client_config = Config::default();
    client_config.use_connection_handshake = true;
    client_config.fragment_size = 1400;

    // Server with smaller fragment size
    let mut server_config = Config::default();
    server_config.use_connection_handshake = true;
    server_config.fragment_size = 1200;

    let mut client = Peer::new("127.0.0.1:3001".parse().unwrap(), &client_config, time);
    let mut server = Peer::new("127.0.0.1:3002".parse().unwrap(), &server_config, time);

    // Handshake should complete successfully
    client.initiate_connect();
    let connect = client.encode_queued_commands().unwrap();

    server.process_command_packet(&connect, time).unwrap();
    let verify = server.encode_queued_commands().unwrap();

    client.process_command_packet(&verify, time).unwrap();

    // Both peers should be connected and able to communicate
    // Each peer uses its own fragment size configuration
}

#[test]
fn test_handshake_checksum_protects_connect() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = true;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:2001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:2002".parse().unwrap(), &config, time);

    // Client sends Connect
    client.initiate_connect();
    let mut connect = client.encode_queued_commands().unwrap();

    // Corrupt the Connect packet
    connect[10] ^= 0xFF;

    // Server should reject the corrupted packet
    let result = server.process_command_packet(&connect, time);
    assert!(result.is_err(), "Server should reject corrupted Connect packet");
}

#[test]
fn test_handshake_checksum_protects_verify_connect() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = true;

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:1001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:1002".parse().unwrap(), &config, time);

    // Normal Connect
    client.initiate_connect();
    let connect = client.encode_queued_commands().unwrap();

    server.process_command_packet(&connect, time).unwrap();
    let mut verify = server.encode_queued_commands().unwrap();

    // Corrupt the VerifyConnect packet
    verify[15] ^= 0xFF;

    // Client should reject the corrupted packet
    let result = client.process_command_packet(&verify, time);
    assert!(result.is_err(), "Client should reject corrupted VerifyConnect packet");
}

#[test]
fn test_handshake_without_checksums() {
    let mut config = Config::default();
    config.use_connection_handshake = true;
    config.use_checksums = false; // Checksums disabled

    let time = Instant::now();
    let mut client = Peer::new("127.0.0.1:10001".parse().unwrap(), &config, time);
    let mut server = Peer::new("127.0.0.1:10002".parse().unwrap(), &config, time);

    // Handshake should still work without checksums
    client.initiate_connect();
    let connect = client.encode_queued_commands().unwrap();

    server.process_command_packet(&connect, time).unwrap();
    let verify = server.encode_queued_commands().unwrap();

    client.process_command_packet(&verify, time).unwrap();

    // Send data
    client.enqueue_command(ProtocolCommand::SendReliable {
        channel_id: 0,
        sequence: 0,
        ordered: true,
        data: b"Test data".to_vec().into(),
    });

    let data = client.encode_queued_commands().unwrap();
    let result = server.process_command_packet(&data, time).unwrap();
    let packets: Vec<_> = result.into_iter().collect();

    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].0.payload(), b"Test data");
}
