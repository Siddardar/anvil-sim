use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Reverse;
use crate::ir::types::*;
use super::eval::eval_wire;

/// An entry in the event queue: (cycle, event_id).
/// Wrapped in Reverse so BinaryHeap acts as a min-heap.
type QueueEntry = Reverse<(isize, isize)>;

pub struct Simulator {
    /// Current register values (what RegRead sees)
    regs: HashMap<String, isize>,
    /// Buffered register writes, applied when cycle advances
    pending_writes: Vec<(String, isize)>,
    /// Min-heap event queue
    heap: BinaryHeap<QueueEntry>,
    /// Wire lookup by id
    wires: HashMap<isize, Wire>,
    /// Event lookup by id
    events: HashMap<isize, Event>,
    /// Cached root event id
    root_id: isize,
    /// Set to true when DebugFinish is hit
    finished: bool,
    /// Hashset to track later events
    later_events: HashSet<isize>,
}

impl Simulator {
    /// Build a simulator from a single-thread IR.
    pub fn new(thread: EventGraph) -> Self {
        let mut heap = BinaryHeap::new();

        let mut events = HashMap::new();
        let mut root_id: isize = -1;
        for event in thread.events {
            let id = event.id;
            if matches!(&event.source, EventSource::RootInit) {
                heap.push(Reverse((0, id)));
                root_id = id;
            }
            events.insert(id, event);
        }

        let mut wires = HashMap::new();
        for wire in thread.wires {
            wires.insert(wire.id, wire);
        }

        let mut regs = HashMap::new();
        for reg in thread.regs {
            let reg_val: isize = 
                reg
                .init
                .as_deref()
                .map(|s| s.parse::<isize>().expect("invalid reg init"))
                .unwrap_or(0);

            regs.insert(reg.name, reg_val);
        }

        Self {
            regs, heap, wires, events, root_id,
            pending_writes: Vec::new(),
            finished: false,
            later_events: HashSet::new(),
        }
    }

    /// Run the simulation until DebugFinish or heap is empty.
    pub fn run(&mut self) {
        let mut prev_cycle: isize = -1;

        while !self.finished {
            let Some(Reverse((cycle, event_id))) = self.heap.pop() else { break };

            if cycle != prev_cycle {
                self.apply_pending_writes();
                prev_cycle = cycle;
            }

            self.fire_event(event_id, cycle);
        }
    }

    /// Fire a single event.
    fn fire_event(&mut self, event_id: isize, cycle: isize) {
        if self.finished { return; }
        self.execute_actions(event_id);
        if self.finished { return; }
        self.schedule_successors(event_id, cycle);
    }

    /// Execute actions for an event.
    fn execute_actions(&mut self, event_id: isize) {
        let event = self.events.get(&event_id).expect("no event found");

        for action in &event.actions {
            match action {
                Action::DebugFinish => {
                    self.finished = true;
                    return;
                },
                Action::DebugPrint { fmt, args } => {
                    let values: Vec<isize> = args.iter()
                        .map(|ld| eval_wire(ld.wire_id.unwrap(), &self.wires, &self.regs))
                        .collect();
                    println!("{}", Self::format_dprint(fmt, &values));
                },
                Action::RegAssign { target, value } => {
                    let val = eval_wire(value.wire_id.unwrap(), &self.wires, &self.regs);
                    
                    let offset = 
                        if target.offset_is_const { 
                            target.offset 
                        } else { 
                            eval_wire(target.offset, &self.wires, &self.regs) 
                        };
                    
                    let mask = ((1isize << target.size) - 1) << offset;                                                                                                
                    let current = *self.regs.get(&target.reg).unwrap_or(&0);                                                                                         
                    let new_val = (current & !mask) | ((val << offset) & mask); 

                    self.pending_writes.push((target.reg.clone(), new_val));
                },
                _ => todo!(),
            }
        }
    }

    /// Schedule successor events based on their source type.
    fn schedule_successors(&mut self, event_id: isize, cycle: isize) {
        let event = self.events.get(&event_id).expect("no event found");
        let outs = event.outs.clone();
        let is_recurse = event.is_recurse;

        for succ_id in outs {
            if self.finished { return; }

            let succ = self.events.get(&succ_id).expect("no successor event found");
            match &succ.source {
                EventSource::SeqCycles { cycles, .. } => {
                    self.heap.push(Reverse((cycle + cycles, succ_id)));
                },
                EventSource::RootBranch { branch_sel, cond_wire_id, branch_cond, .. } => {
                    let cond_val = eval_wire(cond_wire_id.unwrap(), &self.wires, &self.regs);
                    let should_fire = match branch_cond {
                        BranchCond::TrueFalse(_) => {
                            (*branch_sel == 0 && cond_val != 0) || (*branch_sel == 1 && cond_val == 0)
                        },
                        BranchCond::MatchCases { patterns } => {
                            let pat_val = eval_wire(
                                patterns[*branch_sel as usize].wire_id.unwrap(),
                                &self.wires, &self.regs,
                            );
                            cond_val == pat_val
                        },
                    };
                    if should_fire {
                        self.fire_event(succ_id, cycle);
                    }
                },
                EventSource::Branch { .. } => {
                    self.fire_event(succ_id, cycle);
                },
                EventSource::Later { .. } => {
                    if self.later_events.remove(&succ_id) {
                        self.fire_event(succ_id, cycle);
                    } else {
                        self.later_events.insert(succ_id);
                    }
                },
                
                _ => todo!(),
            }
        }

        // if this event is a recurse point, re-fire the root event
        if is_recurse && !self.finished {
            self.fire_event(self.root_id, cycle);
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
