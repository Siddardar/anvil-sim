use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Reverse;
use crate::ir::types::*;
use super::eval::eval_wire;

/// An entry in the event queue: (cycle, event_id, thread_idx).
/// Wrapped in Reverse so BinaryHeap acts as a min-heap.
type QueueEntry = Reverse<(usize, usize, usize)>;

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

/// Shared mutable simulation state (separate from threads for split borrowing)
struct SimState {
    /// Current register values (what RegRead sees)
    regs: HashMap<String, isize>,
    /// Buffered register writes, applied when cycle advances
    pending_writes: Vec<(String, isize)>,
    /// Min-heap event queue
    heap: BinaryHeap<QueueEntry>,
    /// Set to true when DebugFinish is hit
    finished: bool,
}

pub struct Simulator {
    threads: Vec<ThreadState>,
    state: SimState,
}

impl Simulator {
    /// Build a simulator from a single-proc IR.
    pub fn new(proc_threads: Vec<EventGraph>) -> Self {
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

                regs.insert(reg.name, reg_val);
            }

            threads.push(ThreadState {
                wires,
                events,
                root_id,
                later_events: HashSet::new(),
            });
        }

        Self {
            threads,
            state: SimState {
                regs, heap,
                pending_writes: Vec::new(),
                finished: false,
            },
        }
    }

    /// Run the simulation until DebugFinish or heap is empty.
    pub fn run(&mut self) {
        while !self.state.finished {
            let Some(Reverse((cycle, event_id, thread_idx))) = self.state.heap.pop() else { break };

            self.state.fire_event(&mut self.threads[thread_idx], cycle, event_id, thread_idx);
            while let Some(Reverse((next_cycle, _, _))) = self.state.heap.peek() {
                if *next_cycle != cycle { break; }
                let Reverse((_, next_id, next_thread_idx)) = self.state.heap.pop().unwrap();
                self.state.fire_event(&mut self.threads[next_thread_idx], cycle, next_id, next_thread_idx);
            }

            self.state.apply_pending_writes();
        }
    }
}

impl SimState {
    /// Fire a single event.
    fn fire_event(&mut self, thread: &mut ThreadState, cycle: usize, event_id: usize, thread_idx: usize) {
        if self.finished { return; }
        self.execute_actions(thread, event_id);
        if self.finished { return; }
        self.schedule_successors(thread, cycle, event_id, thread_idx);
    }

    /// Execute actions for an event.
    fn execute_actions(&mut self, thread: &ThreadState, event_id: usize) {
        let event = &thread.events[event_id];
        let wires = &thread.wires;

        for action in &event.actions {
            match action {
                Action::DebugFinish => {
                    self.finished = true;
                    return;
                },
                Action::DebugPrint { fmt, args } => {
                    let values: Vec<isize> = args.iter()
                        .map(|ld| eval_wire(ld.wire_id.unwrap(), wires, &self.regs))
                        .collect();
                    println!("{}", Self::format_dprint(fmt, &values));
                },
                Action::RegAssign { target, value } => {
                    let val = eval_wire(value.wire_id.unwrap(), wires, &self.regs);

                    let offset =
                        if target.offset_is_const {
                            target.offset as isize
                        } else {
                            eval_wire(target.offset, wires, &self.regs)
                        };

                    let mask = ((1isize << target.size as isize) - 1) << offset;
                    let current = *self.regs.get(&target.reg).unwrap_or(&0);
                    let new_val = (current & !mask) | ((val << offset) & mask);

                    self.pending_writes.push((target.reg.clone(), new_val));
                },
                _ => todo!(),
            }
        }
    }

    /// Schedule successor events based on their source type.
    fn schedule_successors(&mut self, thread: &mut ThreadState, cycle: usize, event_id: usize, thread_idx: usize) {
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
                    let cond_val = eval_wire(cond_wire_id.unwrap(), &thread.wires, &self.regs);
                    let should_fire = match branch_cond {
                        BranchCond::TrueFalse(_) => {
                            (*branch_sel == 0 && cond_val != 0) || (*branch_sel == 1 && cond_val == 0)
                        },
                        BranchCond::MatchCases { patterns } => {
                            let pat_val = eval_wire(
                                patterns[*branch_sel].wire_id.unwrap(),
                                &thread.wires, &self.regs,
                            );
                            cond_val == pat_val
                        },
                    };
                    if should_fire {
                        self.fire_event(thread, cycle, succ_id, thread_idx);
                    }
                },
                EventSource::Branch { .. } => {
                    self.fire_event(thread, cycle, succ_id, thread_idx);
                },
                EventSource::Later { .. } => {
                    if thread.later_events.remove(&succ_id) {
                        self.fire_event(thread, cycle, succ_id, thread_idx);
                    } else {
                        thread.later_events.insert(succ_id);
                    }
                },

                _ => todo!(),
            }
        }

        // if this event is a recurse point, re-fire the root event
        if is_recurse && !self.finished {
            self.fire_event(thread, cycle, thread.root_id, thread_idx);
        }
    }

    /// Apply pending register writes.
    fn apply_pending_writes(&mut self) {
        for (reg_name, val) in self.pending_writes.drain(..) {
            self.regs.insert(reg_name, val);
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
