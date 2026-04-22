use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::{Arc, Mutex, Condvar, atomic::{AtomicBool, Ordering}};
use std::cmp::Reverse;
use crate::ir::types::*;
use super::eval::eval_wire;

/// An entry in the event queue: (cycle, event_id, thread_idx).
/// Wrapped in Reverse so BinaryHeap acts as a min-heap.
type QueueEntry = Reverse<(usize, usize, usize)>;

/// A channel event that couldn't complete immediately (non-blocking parking)
struct ParkedEvent {
    succ_id: usize,
    thread_idx: usize,
    cycle: usize,
    kind: ParkedKind,
}

enum ParkedKind {
    Recv { endpoint: String, msg: String },
    Send { endpoint: String, msg: String, value: isize },
}

/// Holds all the wires and events associated with a thread
struct ThreadState {
    /// Wire lookup by id
    wires: Vec<Wire>,
    /// Event lookup by id
    events: Vec<Event>,
    /// Cached root event id
    root_id: usize,
    /// Hashset to track later events
    later_events: HashSet<usize>,
}

pub struct SharedChannel {
    pub data: HashMap<String, Option<isize>>, // msg name -> val
}

pub struct ChannelHandler {
    pub inner: Mutex<SharedChannel>,
    pub condvar: Condvar,
}

pub type ChannelTable = HashMap<String, Arc<ChannelHandler>>;
pub type GlobalFinished = Arc<AtomicBool>;

/// Read `len` bits from a byte array starting at bit `offset`, returned as isize (max 64 bits).
pub fn read_reg_bits(data: &[u8], offset: usize, len: usize) -> isize {
    let mut result: isize = 0;
    for i in 0..len.min(64) {
        let bit_pos = offset + i;
        let byte_idx = bit_pos / 8;
        let bit_idx = bit_pos % 8;
        if byte_idx < data.len() && data[byte_idx] & (1 << bit_idx) != 0 {
            result |= 1isize << i;
        }
    }
    result
}

/// Write `len` bits of `value` into a byte array at bit `offset`.
fn write_reg_bits(data: &mut Vec<u8>, offset: usize, len: usize, value: isize) {
    let max_byte = (offset + len + 7) / 8;
    if data.len() < max_byte {
        data.resize(max_byte, 0);
    }
    for i in 0..len {
        let bit_pos = offset + i;
        let byte_idx = bit_pos / 8;
        let bit_idx = bit_pos % 8;
        if i < 64 && value & (1isize << i) != 0 {
            data[byte_idx] |= 1u8 << bit_idx;
        } else {
            data[byte_idx] &= !(1u8 << bit_idx);
        }
    }
}

/// Shared mutable simulation state (separate from threads for split borrowing)
struct SimState {
    /// Current register values stored as byte arrays (supports arbitrary widths)
    regs: HashMap<String, Vec<u8>>,
    /// Buffered register writes: (reg_name, bit_offset, bit_size, value)
    pending_writes: Vec<(String, usize, usize, isize)>,
    /// Min-heap event queue
    heap: BinaryHeap<QueueEntry>,
    /// Set to true when DebugFinish is hit
    finished: bool,
    /// Events waiting on channel data
    parked: Vec<ParkedEvent>,
    /// Cached values from SeqRecv events so downstream wires can still read them
    /// after the channel slot is cleared. Keyed by (endpoint, msg).
    recv_cache: HashMap<(String, String), isize>,
}

pub struct Simulator {
    proc_name: String,
    channel_table: Arc<ChannelTable>,
    global_finished: GlobalFinished,
    max_cycles: Option<usize>,
    threads: Vec<ThreadState>,
    state: SimState,
}

impl Simulator {
    /// Build a simulator from a single-proc IR.
    pub fn new (proc_name: String, proc_threads: Vec<EventGraph>, channel_table: Arc<ChannelTable>, global_finished: GlobalFinished, max_cycles: Option<usize>) -> Self {
        let mut heap = BinaryHeap::new();

        let mut regs = HashMap::new();

        let mut threads: Vec<ThreadState> = Vec::new();
        for (idx, thread) in proc_threads.into_iter().enumerate() {
            let mut events = thread.events;
            events.sort_by_key(|e| e.id);

            let root_id = events.iter()
                .find(|e| matches!(&e.source, EventSource::RootInit))
                .expect("no root event").id;

            heap.push(Reverse((0, root_id, idx)));

            let mut wires = thread.wires;
            wires.sort_by_key(|w| w.id);

            for reg in thread.regs {
                let reg_val: isize =
                    reg
                    .init
                    .as_deref()
                    .map(|s| s.parse::<isize>().expect("invalid reg init"))
                    .unwrap_or(0);

                let entry = regs.entry(reg.name).or_insert_with(Vec::new);
                if reg_val != 0 {
                    write_reg_bits(entry, 0, 64, reg_val);
                }
            }

            threads.push(ThreadState {
                wires,
                events,
                root_id,
                later_events: HashSet::new(),
            });
        }

        Self {
            proc_name,
            channel_table,
            global_finished,
            max_cycles,
            threads,
            state: SimState {
                regs, heap,
                pending_writes: Vec::new(),
                finished: false,
                parked: Vec::new(),
                recv_cache: HashMap::new(),
            },
        }
    }

    /// Run the simulation until DebugFinish or heap is empty.
    pub fn run(&mut self) {
        while !self.state.finished && !self.global_finished.load(Ordering::SeqCst) {
            let Some(Reverse((cycle, event_id, thread_idx))) = self.state.heap.pop() else { break };

            if let Some(max) = self.max_cycles {
                if cycle >= max {
                    self.global_finished.store(true, Ordering::SeqCst);
                    for handler in self.channel_table.values() {
                        handler.condvar.notify_all();
                    }
                    break;
                }
            }

            self.state.fire_event(&mut self.threads[thread_idx], cycle, event_id, thread_idx, &self.proc_name, &self.channel_table, &self.global_finished);
            loop {
                self.state.try_unpark(&self.channel_table, &self.global_finished);
                match self.state.heap.peek() {
                    Some(Reverse((next_cycle, _, _))) if *next_cycle == cycle => {
                        let Reverse((_, next_id, next_thread_idx)) = self.state.heap.pop().unwrap();
                        self.state.fire_event(&mut self.threads[next_thread_idx], cycle, next_id, next_thread_idx, &self.proc_name, &self.channel_table, &self.global_finished);
                    }
                    _ => break,
                }
            }

            self.state.apply_pending_writes();
        }
    }
}

impl SimState {
    /// Fire a single event.
    fn fire_event(
        &mut self, thread: &mut ThreadState, cycle: usize, event_id: usize,
        thread_idx: usize, proc_name: &str, channel_table: &ChannelTable,
        global_finished: &GlobalFinished) {

        if self.finished { return; }

        // Snapshot received channel data into recv_cache before executing.
        // This lets downstream wire evaluations (including from parked events
        // that fire later) access the received value after the channel is cleared.
        if let EventSource::SeqRecv { endpoint, msg, .. } = &thread.events[event_id].source {
            let handler = channel_table.get(endpoint)
                .unwrap_or_else(|| panic!("no channel for endpoint '{}'", endpoint));
            let ch = handler.inner.lock().unwrap();
            if let Some(Some(val)) = ch.data.get(msg) {
                self.recv_cache.insert((endpoint.clone(), msg.clone()), *val);
            }
        }

        self.execute_actions(thread, event_id, proc_name, channel_table, global_finished);
        if self.finished { return; }

        self.schedule_successors(thread, cycle, event_id, thread_idx, proc_name, channel_table, global_finished);

        // After a SeqRecv event's actions have read the data, clear the channel
        // so the sender can send again on the next iteration
        if let EventSource::SeqRecv { endpoint, msg, .. } = &thread.events[event_id].source {
            let handler = channel_table.get(endpoint)
                .unwrap_or_else(|| panic!("no channel for endpoint '{}'", endpoint));
            let mut ch = handler.inner.lock().unwrap();
            ch.data.insert(msg.clone(), None);
            drop(ch);
            handler.condvar.notify_all();
        }
    }

    /// Execute actions for an event.
    /// ImmediateSend actions run first so channel data is available
    /// before any MessagePort wire evaluations block.
    fn execute_actions(&mut self, thread: &ThreadState, event_id: usize, proc_name: &str, channel_table: &ChannelTable, global_finished: &GlobalFinished) {
        let event = &thread.events[event_id];
        let wires = &thread.wires;

        // execute all ImmediateSend actions first
        for action in &event.actions {
            if let Action::ImmediateSend { endpoint, msg, value } = action {
                let val = eval_wire(value.wire_id.unwrap(), wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache);
                let handler = channel_table.get(endpoint)
                    .unwrap_or_else(|| panic!("no channel for endpoint '{}'", endpoint));
                let mut ch = handler.inner.lock().unwrap();
                while ch.data.get(msg).copied().flatten().is_some() {
                    if global_finished.load(Ordering::SeqCst) { return; }
                    ch = handler.condvar.wait(ch).unwrap();
                }
                ch.data.insert(msg.clone(), Some(val));
                handler.condvar.notify_all();
            }
        }

        // Phase 2: execute everything except ImmediateSend and ImmediateRecv
        for action in &event.actions {
            match action {
                Action::ImmediateSend { .. } | Action::ImmediateRecv { .. } => {},
                Action::DebugFinish => {
                    self.finished = true;
                    global_finished.store(true, Ordering::SeqCst);
                    for handler in channel_table.values() {
                        handler.condvar.notify_all();
                    }
                    return;
                },
                Action::DebugPrint { fmt, args } => {
                    let values: Vec<isize> = args.iter()
                        .map(|ld| eval_wire(ld.wire_id.unwrap(), wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache))
                        .collect();
                    println!("{}", Self::format_dprint(fmt, &values));
                },
                Action::RegAssign { target, value } => {
                    let val = eval_wire(value.wire_id.unwrap(), wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache);

                    let offset =
                        if target.offset_is_const {
                            target.offset
                        } else {
                            eval_wire(target.offset, wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache) as usize
                        };

                    self.pending_writes.push((target.reg.clone(), offset, target.size, val));
                },
                _ => todo!(),
            }
        }

        // Phase 3: execute all ImmediateRecv last (after MessagePort reads)
        for action in &event.actions {
            if let Action::ImmediateRecv { endpoint, msg } = action {
                let handler = channel_table.get(endpoint)
                    .unwrap_or_else(|| panic!("no channel for endpoint '{}'", endpoint));
                let mut ch = handler.inner.lock().unwrap();
                ch.data.insert(msg.clone(), None);
                handler.condvar.notify_all();
            }
        }
    }

    /// Schedule successor events based on their source type.
    fn schedule_successors(&mut self, thread: &mut ThreadState, cycle: usize, event_id: usize, thread_idx: usize, proc_name: &str, channel_table: &ChannelTable, global_finished: &GlobalFinished) {
        let outs = thread.events[event_id].outs.clone();
        let is_recurse = thread.events[event_id].is_recurse;

        for succ_id in outs {
            if self.finished { return; }

            let succ = &thread.events[succ_id];
            match &succ.source {
                EventSource::SeqCycles { cycles, .. } => {
                    self.heap.push(Reverse((cycle + *cycles, succ_id, thread_idx)));
                },
                EventSource::RootBranch { branch_sel, cond_wire_id, branch_cond, .. } => {
                    let cond_val = eval_wire(cond_wire_id.unwrap(), &thread.wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache);
                    let should_fire = match branch_cond {
                        BranchCond::TrueFalse(_) => {
                            (*branch_sel == 0 && cond_val != 0) || (*branch_sel == 1 && cond_val == 0)
                        },
                        BranchCond::MatchCases { patterns } => {
                            let pat_val = eval_wire(
                                patterns[*branch_sel].wire_id.unwrap(),
                                &thread.wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache,
                            );
                            cond_val == pat_val
                        },
                    };
                    if should_fire {
                        self.fire_event(thread, cycle, succ_id, thread_idx, proc_name, channel_table, global_finished);
                    }
                },
                EventSource::Branch { .. } => {
                    self.fire_event(thread, cycle, succ_id, thread_idx, proc_name, channel_table, global_finished);
                },
                EventSource::Later { .. } => {
                    if thread.later_events.remove(&succ_id) {
                        self.fire_event(thread, cycle, succ_id, thread_idx, proc_name, channel_table, global_finished);
                    } else {
                        thread.later_events.insert(succ_id);
                    }
                },
                EventSource::SeqSend { endpoint, msg, .. } => {
                    // Find the SustainedSend whose until_id matches this SeqSend event
                    let wire_id = thread.events.iter()
                        .flat_map(|e| e.sustained_actions.iter())
                        .find_map(|sa| match sa {
                            SustainedAction::SustainedSend { until_id, endpoint: ep, msg: m, value }
                                if *until_id == succ_id && ep == endpoint && m == msg => Some(value.wire_id.unwrap()),
                            _ => None
                        })
                        .expect("no SustainedSend for SeqSend");

                    let val = eval_wire(wire_id, &thread.wires, &self.regs, proc_name, channel_table, global_finished, &self.recv_cache);
                    let handler = channel_table.get(endpoint)
                        .unwrap_or_else(|| panic!("no channel for endpoint {}", endpoint));

                    let ch = handler.inner.lock().unwrap();
                    if ch.data.get(msg).copied().flatten().is_none() {
                        let mut ch = ch;
                        ch.data.insert(msg.clone(), Some(val));
                        drop(ch);
                        handler.condvar.notify_all();
                        self.fire_event(thread, cycle, succ_id, thread_idx, proc_name, channel_table, global_finished);
                    } else {
                        drop(ch);
                        self.parked.push(ParkedEvent {
                            succ_id, thread_idx, cycle,
                            kind: ParkedKind::Send { endpoint: endpoint.clone(), msg: msg.clone(), value: val },
                        });
                    }
                },
                EventSource::SeqRecv { endpoint, msg, .. } => {
                    let handler = channel_table.get(endpoint)
                        .unwrap_or_else(|| panic!("no channel for endpoint {}", endpoint));

                    let ch = handler.inner.lock().unwrap();
                    if ch.data.get(msg).copied().flatten().is_some() {
                        drop(ch);
                        self.fire_event(thread, cycle, succ_id, thread_idx, proc_name, channel_table, global_finished);
                    } else {
                        drop(ch);
                        self.parked.push(ParkedEvent {
                            succ_id, thread_idx, cycle,
                            kind: ParkedKind::Recv { endpoint: endpoint.clone(), msg: msg.clone() },
                        });
                    }
                }
                _ => todo!(),
            }
        }

        // if this event is a recurse point, re-fire the root event
        if is_recurse && !self.finished {
            self.fire_event(thread, cycle, thread.root_id, thread_idx, proc_name, channel_table, global_finished);
        }
    }

    /// Check parked events and re-enqueue any that can now proceed.
    fn try_unpark(&mut self, channel_table: &ChannelTable, global_finished: &GlobalFinished) {
        let mut made_progress = true;
        while made_progress {
            made_progress = false;
            let mut i = 0;
            while i < self.parked.len() {
                if global_finished.load(Ordering::SeqCst) { return; }
                let ready = match &self.parked[i].kind {
                    ParkedKind::Recv { endpoint, msg } => {
                        let handler = channel_table.get(endpoint).unwrap();
                        let ch = handler.inner.lock().unwrap();
                        ch.data.get(msg).copied().flatten().is_some()
                    }
                    ParkedKind::Send { endpoint, msg, .. } => {
                        let handler = channel_table.get(endpoint).unwrap();
                        let ch = handler.inner.lock().unwrap();
                        ch.data.get(msg).copied().flatten().is_none()
                    }
                };
                if ready {
                    let p = self.parked.remove(i);
                    if let ParkedKind::Send { ref endpoint, ref msg, value } = p.kind {
                        let handler = channel_table.get(endpoint).unwrap();
                        let mut ch = handler.inner.lock().unwrap();
                        ch.data.insert(msg.clone(), Some(value));
                        drop(ch);
                        handler.condvar.notify_all();
                    }
                    self.heap.push(Reverse((p.cycle, p.succ_id, p.thread_idx)));
                    made_progress = true;
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Apply pending register writes.
    fn apply_pending_writes(&mut self) {
        for (reg_name, offset, size, val) in self.pending_writes.drain(..) {
            let data = self.regs.entry(reg_name).or_insert_with(Vec::new);
            write_reg_bits(data, offset, size, val);
        }
    }

    fn format_dprint(fmt: &str, values: &[isize]) -> String {
        let mut res = String::new();
        let mut val_idx = 0;
        let mut chars = fmt.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('d') => {
                        res.push_str(&format!("{:>2}", values[val_idx]));
                        val_idx += 1;
                    }
                    Some(other) => {
                        res.push('%');
                        res.push(other);
                    }
                    None => res.push('%')
                }
            } else {
                res.push(c)
            }
        }
        res
    }
}
