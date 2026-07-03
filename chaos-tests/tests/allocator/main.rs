// Allocator workload tests: run with
//   cargo test --test allocator -- --nocapture
// to see HeuristicAllocator split/merge/exact statistics.
use chaos_tests::*;
use std::time::Instant;

const BASE: usize = 0x1000_0000;

fn report(name: &str, elapsed: std::time::Duration, stats: &BuddyStatistics, heuristic: &HeuristicStatistics, final_free: usize, final_largest: usize, final_frag: usize) {
    println!(
        "[{}] elapsed={:?} split={} merge={} exact={} failed={} final={}/{}/{} preserve={} coalesce={} pressure={}/{}",
        name,
        elapsed,
        stats.split_count,
        stats.merge_count,
        stats.exact_hit_count,
        stats.failed_alloc_count,
        final_free,
        final_largest,
        final_frag,
        heuristic.preserve_count,
        heuristic.active_coalesce_count,
        heuristic.pressure_enter_count,
        heuristic.pressure_exit_count,
    );
}

#[test]
fn workload_small_page_churn() {
    println!("\n=== small_page_churn ===");

    let mut buddy = BuddyAllocator::new(BASE, 1024, 10);
    let t0 = Instant::now();
    for _ in 0..64 {
        let addr = buddy.alloc_page().unwrap();
        buddy.free(addr);
    }
    let buddy_elapsed = t0.elapsed();
    println!(
        "[buddy] elapsed={:?} split={} merge={} exact={} failed={} final=1024/10/0",
        buddy_elapsed, buddy.statistics.split_count, buddy.statistics.merge_count,
        buddy.statistics.exact_hit_count, buddy.statistics.failed_alloc_count,
    );

    let mut alloc = HeuristicAllocator::new(BASE, 1024, 10);
    let t1 = Instant::now();
    for _ in 0..64 {
        let addr = alloc.alloc_page().unwrap();
        alloc.free(addr);
    }
    let heuristic_elapsed = t1.elapsed();
    let heuristic = alloc.heuristic_statistics();
    report(
        "heuristic",
        heuristic_elapsed,
        &alloc.statistics,
        &heuristic,
        alloc.free_pages_count(),
        alloc.largest_free_order(),
        alloc.fragmentation_score(),
    );

    assert_eq!(alloc.free_pages_count(), 1024);
    assert_eq!(alloc.statistics.failed_alloc_count, 0);
    assert!(heuristic.preserve_count > 0);
    assert!(alloc.statistics.split_count < buddy.statistics.split_count);
}

#[test]
fn workload_mixed_order_churn() {
    println!("\n=== mixed_order_churn ===");

    let orders = [0usize, 0, 1, 2, 0, 3, 1, 0, 2, 1, 0, 0, 3, 2, 1, 0];
    let mut live: Vec<usize> = Vec::new();

    let mut buddy = BuddyAllocator::new(BASE, 2048, 11);
    let t0 = Instant::now();
    for i in 0..256 {
        let order = if i % 17 == 0 { buddy.alloc_order_aligned(1, 4) } else { buddy.alloc_order(orders[i % orders.len()]) };
        live.push(order.unwrap());
        if i % 3 == 2 {
            let oldest = live.remove(0);
            buddy.free(oldest);
        }
    }
    for addr in live.drain(..) {
        buddy.free(addr);
    }
    let buddy_elapsed = t0.elapsed();
    println!(
        "[buddy] elapsed={:?} split={} merge={} exact={} failed={} final=2048/11/0",
        buddy_elapsed, buddy.statistics.split_count, buddy.statistics.merge_count,
        buddy.statistics.exact_hit_count, buddy.statistics.failed_alloc_count,
    );

    let mut live: Vec<usize> = Vec::new();
    let mut alloc = HeuristicAllocator::new(BASE, 2048, 11);
    let t1 = Instant::now();
    for i in 0..256 {
        let order = if i % 17 == 0 { alloc.alloc_order_aligned(1, 4) } else { alloc.alloc_order(orders[i % orders.len()]) };
        live.push(order.unwrap());
        if i % 3 == 2 {
            let oldest = live.remove(0);
            alloc.free(oldest);
        }
    }
    for addr in live.drain(..) {
        alloc.free(addr);
    }
    let heuristic_elapsed = t1.elapsed();
    let heuristic = alloc.heuristic_statistics();
    report(
        "heuristic",
        heuristic_elapsed,
        &alloc.statistics,
        &heuristic,
        alloc.free_pages_count(),
        alloc.largest_free_order(),
        alloc.fragmentation_score(),
    );

    assert_eq!(alloc.free_pages_count(), 2048);
    assert_eq!(alloc.statistics.failed_alloc_count, 0);
}

#[test]
fn workload_large_block_pressure() {
    println!("\n=== large_block_pressure ===");

    let mut buddy = BuddyAllocator::new(BASE, 64, 6);
    let mut live: Vec<usize> = Vec::new();
    let t0 = Instant::now();
    for _ in 0..64 {
        live.push(buddy.alloc_page().unwrap());
    }
    for i in (0..64).step_by(2) {
        buddy.free(live[i]);
    }
    let _failed = buddy.alloc_order(5);
    for i in (1..64).step_by(2) {
        buddy.free(live[i]);
    }
    let buddy_elapsed = t0.elapsed();
    println!(
        "[buddy] elapsed={:?} split={} merge={} exact={} failed={} final=64/6/0",
        buddy_elapsed, buddy.statistics.split_count, buddy.statistics.merge_count,
        buddy.statistics.exact_hit_count, buddy.statistics.failed_alloc_count,
    );

    let mut alloc = HeuristicAllocator::new(BASE, 64, 6);
    let mut live: Vec<usize> = Vec::new();
    let t1 = Instant::now();
    for _ in 0..64 {
        live.push(alloc.alloc_page().unwrap());
    }
    for i in (0..64).step_by(2) {
        alloc.free(live[i]);
    }
    let failed = alloc.alloc_order(5);
    assert!(failed.is_none());
    assert!(alloc.merge_pressure);
    for i in (1..64).step_by(2) {
        alloc.free(live[i]);
    }
    let heuristic_elapsed = t1.elapsed();
    let heuristic = alloc.heuristic_statistics();
    report(
        "heuristic",
        heuristic_elapsed,
        &alloc.statistics,
        &heuristic,
        alloc.free_pages_count(),
        alloc.largest_free_order(),
        alloc.fragmentation_score(),
    );

    assert_eq!(alloc.free_pages_count(), 64);
    assert_eq!(alloc.largest_free_order(), 6);
    assert_eq!(alloc.fragmentation_score(), 0);
    assert_eq!(alloc.statistics.failed_alloc_count, 1);
    assert_eq!(heuristic.pressure_enter_count, 1);
    assert_eq!(heuristic.pressure_exit_count, 1);
    assert!(!alloc.merge_pressure);
}
