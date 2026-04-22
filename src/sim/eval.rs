use std::collections::HashMap;
use crate::ir::types::*;
use super::engine::{ChannelTable, GlobalFinished, read_reg_bits};

/// Evaluate a wire by id, recursively resolving dependencies.
/// `wires` maps wire id -> Wire definition.
/// `regs` maps register name -> current value (byte array).
pub fn eval_wire(
    wire_id: usize,
    wires: &Vec<Wire>,
    regs: &HashMap<String, Vec<u8>>,
    proc_name: &str,
    channel_table: &ChannelTable,
    global_finished: &GlobalFinished,
    recv_cache: &HashMap<(String, String), isize>,
) -> isize {
    let current_wire = wires.get(wire_id).expect("unknown wire id");
    let raw = match &current_wire.source {
        WireSource::Literal { value, width:_ } => *value,

        WireSource::RegRead { reg } => {
            let data = regs.get(reg).map(|v| v.as_slice()).unwrap_or(&[]);
            read_reg_bits(data, 0, current_wire.size.min(64))
        },

        WireSource::Binary { op, left, right } => {
            let left_val = eval_wire(*left, wires, regs, proc_name, channel_table, global_finished, recv_cache);
            // TODO: right can also be an array of ids where you check if left val
            // is equal to one of the wire ids vals on the right.
            let right_id = right.as_u64().expect("expected single wire id") as usize;
            let right_val = eval_wire(right_id, wires, regs, proc_name, channel_table, global_finished, recv_cache);
            eval_bin_op(op, left_val, right_val)
        },

        WireSource::Unary { op, operand } => {
            let operand_wire = wires.get(*operand).expect("unknown wire id");
            let operand_size = operand_wire.size;
            let operand_val = eval_wire(*operand, wires, regs, proc_name, channel_table, global_finished, recv_cache);
            eval_un_op(op, operand_val, operand_size)
        },

        WireSource::Switch { cases, default } => {
            for case in cases {
                let cond_val = eval_wire(case.cond, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                if cond_val != 0 {
                    return eval_wire(case.val, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                }
            }
            eval_wire(*default, wires, regs, proc_name, channel_table, global_finished, recv_cache)
        },

        WireSource::Cases { value, cases, default } => {
            let match_val = eval_wire(*value, wires, regs, proc_name, channel_table, global_finished, recv_cache);
            for case in cases {
                let pat_val = eval_wire(case.cond, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                if match_val == pat_val {
                    return eval_wire(case.val, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                }
            }
            eval_wire(*default, wires, regs, proc_name, channel_table, global_finished, recv_cache)
        },

        WireSource::Concat { wires: wire_ids } => {
            let mut result: isize = 0;
            for &wid in wire_ids {
                let w = wires.get(wid).expect("unknown wire id");
                let val = eval_wire(wid, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                if w.size >= 64 {
                    result = val;
                } else {
                    result = (result << w.size) | (val & ((1 << w.size) - 1));
                }
            }
            result
        },

        WireSource::Slice { wire, offset, len, offset_is_const } => {
            let off = if *offset_is_const {
                *offset
            } else {
                eval_wire(*offset, wires, regs, proc_name, channel_table, global_finished, recv_cache) as usize
            };

            // For RegRead inputs, read directly from byte array (handles wide registers)
            if let WireSource::RegRead { reg } = &wires[*wire].source {
                let data = regs.get(reg).map(|v| v.as_slice()).unwrap_or(&[]);
                read_reg_bits(data, off, *len)
            } else {
                let wire_val = eval_wire(*wire, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                if off >= 64 {
                    0
                } else if *len >= 64 {
                    wire_val >> off
                } else {
                    (wire_val >> off) & ((1isize << len) - 1)
                }
            }
        },

        WireSource::Update { base, updates } => {
            let mut result = eval_wire(*base, wires, regs, proc_name, channel_table, global_finished, recv_cache);
            for update in updates {
                let val = eval_wire(update.wire, wires, regs, proc_name, channel_table, global_finished, recv_cache);
                if update.offset + update.size <= 64 {
                    let mask = ((1isize << update.size) - 1) << update.offset;
                    result = (result & !mask) | ((val << update.offset) & mask);
                }
            }
            result
        },

        WireSource::MessagePort { endpoint, msg, .. } => {
            // Check recv_cache first — the channel slot may have been cleared
            // after the SeqRecv fired, but the cached value is still valid.
            if let Some(&cached) = recv_cache.get(&(endpoint.clone(), msg.clone())) {
                cached
            } else {
                let handler = channel_table.get(endpoint)
                    .unwrap_or_else(|| panic!("no channel for endpoint '{}'", endpoint));
                let mut ch = handler.inner.lock().unwrap();
                while ch.data.get(msg).copied().flatten().is_none() {
                    if global_finished.load(std::sync::atomic::Ordering::SeqCst) {
                        return 0;
                    }
                    ch = handler.condvar.wait(ch).unwrap();
                }
                ch.data[msg].unwrap()
            }
        },
        WireSource::MessageValidPort { endpoint, msg} => {
            let handler = channel_table.get(endpoint)
                .unwrap_or_else(|| panic!("no channel for endpoint {}", endpoint));

            let ch = handler.inner.lock().unwrap();
            if ch.data.get(msg).copied().flatten().is_some() { 1 } else { 0 }
        },
        WireSource::MessageAckPort { .. } => todo!("MessageAckPort not yet supported"),
    };

    if current_wire.size >= 64 { raw } else { raw & ((1isize << current_wire.size) - 1) }

}

fn eval_bin_op(op: &BinOp, left: isize, right: isize) -> isize {
    match op {
        BinOp::Add => left.wrapping_add(right),
        BinOp::Sub => left.wrapping_sub(right),
        BinOp::Mul => left.wrapping_mul(right),
        BinOp::Xor => left ^ right,
        BinOp::And => left & right,
        BinOp::Or  => left | right,
        BinOp::LAnd => (left != 0 && right != 0) as isize,
        BinOp::LOr  => (left != 0 || right != 0) as isize,
        BinOp::Lt  => (left < right) as isize,
        BinOp::Gt  => (left > right) as isize,
        BinOp::Lte => (left <= right) as isize,
        BinOp::Gte => (left >= right) as isize,
        BinOp::Shl => if right >= 64 { 0 } else { left << right },
        BinOp::Shr => if right >= 64 { 0 } else { left >> right },
        BinOp::Eq  => (left == right) as isize,
        BinOp::Neq => (left != right) as isize,
        BinOp::Inside => todo!(),
    }
}

fn eval_un_op(op: &UnOp, val: isize, current_wire_size: usize) -> isize {
    match op {
        UnOp::Neg => -val,
        UnOp::Not => !val,
        UnOp::AndAll => {
            if current_wire_size >= 64 {
                (val == !0isize) as isize
            } else {
                (val == ((1 << current_wire_size) - 1)) as isize
            }
        },
        UnOp::OrAll => (val != 0) as isize
    }
}
