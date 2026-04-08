use std::collections::HashMap;
use crate::ir::types::*;
use super::engine::{ChannelTable, GlobalFinished};

/// Evaluate a wire by id, recursively resolving dependencies.
/// `wires` maps wire id -> Wire definition.
/// `regs` maps register name -> current value.
pub fn eval_wire(
    wire_id: usize,
    wires: &Vec<Wire>,
    regs: &HashMap<String, isize>,
    proc_name: &str,
    channel_table: &ChannelTable,
    global_finished: &GlobalFinished,
) -> isize {
    let current_wire = wires.get(wire_id).expect("unknown wire id");
    let raw = match &current_wire.source {
        WireSource::Literal { value, width:_ } => *value,
        
        WireSource::RegRead { reg } => *regs.get(reg).expect("reg not found"),
        
        WireSource::Binary { op, left, right } => {
            let left_val = eval_wire(*left, wires, regs, proc_name, channel_table, global_finished);
            // TODO: right can also be an array of ids where you check if left val
            // is equal to one of the wire ids vals on the right. 
            let right_id = right.as_u64().expect("expected single wire id") as usize;
            let right_val = eval_wire(right_id, wires, regs, proc_name, channel_table, global_finished);
            eval_bin_op(op, left_val, right_val)
        },

        WireSource::Unary { op, operand } => {
            let operand_wire = wires.get(*operand).expect("unknown wire id");
            let operand_size = operand_wire.size;
            let operand_val = eval_wire(*operand, wires, regs, proc_name, channel_table, global_finished);
            eval_un_op(op, operand_val, operand_size)
        },

        WireSource::Switch { cases, default } => {
            for case in cases {
                let cond_val = eval_wire(case.cond, wires, regs, proc_name, channel_table, global_finished);
                if cond_val != 0 {
                    return eval_wire(case.val, wires, regs, proc_name, channel_table, global_finished);
                }
            }
            eval_wire(*default, wires, regs, proc_name, channel_table, global_finished)
        },

        WireSource::Cases { value, cases, default } => {
            let match_val = eval_wire(*value, wires, regs, proc_name, channel_table, global_finished);
            for case in cases {
                let pat_val = eval_wire(case.cond, wires, regs, proc_name, channel_table, global_finished);
                if match_val == pat_val {
                    return eval_wire(case.val, wires, regs, proc_name, channel_table, global_finished);
                }
            }
            eval_wire(*default, wires, regs, proc_name, channel_table, global_finished)
        },

        WireSource::Concat { wires: wire_ids } => {
            let mut result: isize = 0;
            for &wid in wire_ids {
                let w = wires.get(wid).expect("unknown wire id");
                let val = eval_wire(wid, wires, regs, proc_name, channel_table, global_finished);
                result = (result << w.size) | (val & ((1 << w.size) - 1));
            }
            result
        },

        WireSource::Slice { wire, offset, len } => {
            let wire_val = eval_wire(*wire, wires, regs, proc_name, channel_table, global_finished);
            let off = if let Some(n) = offset.as_i64() {
                n as isize
            } else {
                // offset is a wire id, evaluate it
                eval_wire(offset.as_i64().unwrap() as usize, wires, regs, proc_name, channel_table, global_finished)
            };
            // shift right by offset, then mask to len bits
            (wire_val >> off) & ((1 << len) - 1)
        },

        WireSource::Update { base, updates } => {
            let mut result = eval_wire(*base, wires, regs, proc_name, channel_table, global_finished);
            for update in updates {
                let val = eval_wire(update.wire, wires, regs, proc_name, channel_table, global_finished);
                let mask = ((1isize << update.size) - 1) << update.offset;
                // clear the target bits, then set them
                result = (result & !mask) | ((val << update.offset) & mask);
            }
            result
        },

        WireSource::MessagePort { .. } => todo!("MessagePort not yet supported"),
        WireSource::MessageValidPort { .. } => todo!("MessageValidPort not yet supported"),
        WireSource::MessageAckPort { .. } => todo!("MessageAckPort not yet supported"),
    };

    raw & ((1isize << current_wire.size) - 1)

}

fn eval_bin_op(op: &BinOp, left: isize, right: isize) -> isize {
    match op {
        BinOp::Add => left + right,
        BinOp::Sub => left - right,
        BinOp::Mul => left * right,
        BinOp::Xor => left ^ right,
        BinOp::And => left & right,
        BinOp::Or  => left | right,
        BinOp::LAnd => (left != 0 && right != 0) as isize,
        BinOp::LOr  => (left != 0 || right != 0) as isize,
        BinOp::Lt  => (left < right) as isize,
        BinOp::Gt  => (left > right) as isize,
        BinOp::Lte => (left <= right) as isize,
        BinOp::Gte => (left >= right) as isize,
        BinOp::Shl => left << right,
        BinOp::Shr => left >> right,
        BinOp::Eq  => (left == right) as isize,
        BinOp::Neq => (left != right) as isize,
        BinOp::Inside => todo!(), // part of the TODO on top
    }
}

fn eval_un_op(op: &UnOp, val: isize, current_wire_size: usize) -> isize {
    match op {
        UnOp::Neg => -val,
        UnOp::Not => !val,
        UnOp::AndAll => (val == ((1 << current_wire_size) - 1)) as isize, // checks if all bits are 1
        UnOp::OrAll => (val != 0) as isize
    }
}