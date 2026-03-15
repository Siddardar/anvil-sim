use std::collections::HashMap;
use crate::ir::types::*;

/// Evaluate a wire by id, recursively resolving dependencies.
/// `wires` maps wire id -> Wire definition.
/// `regs` maps register name -> current value.
pub fn eval_wire(
    wire_id: isize,
    wires: &HashMap<isize, Wire>,
    regs: &HashMap<String, isize>,
) -> isize {
    let current_wire = wires.get(&wire_id).expect("unknown wire id");
    match current_wire.source {
        WireSource::Literal { value, width:_ } => value,
        WireSource::RegRead { ref reg } => *regs.get(reg).expect("reg not found"),
        WireSource::Binary { ref op, left, ref right } => {
            let left_val = eval_wire(left, wires, regs);
            // TODO: right can also be an array of ids where you check if left val
            // is equal to one of the wire ids vals on the right. 
            let right_id = right.as_i64().expect("expected single wire id") as isize;
            let right_val = eval_wire(right_id, wires, regs);
            eval_bin_op(op, left_val, right_val)
        },
        _ => todo!()
    }
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