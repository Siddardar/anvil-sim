use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct Collection {
    pub procs: Vec<ProcGraph>,
}

#[derive(Deserialize, Serialize)]
pub struct ProcGraph {
    pub name: String,
    pub threads: Vec<EventGraph>,
}

#[derive(Deserialize, Serialize)]
pub struct EventGraph {
    pub thread_id: isize,
    pub is_general_recursive: bool,
    pub comb: bool,
    pub events: Vec<Event>,
    pub wires: Vec<Wire>,
    pub regs: Vec<RegDef>,
}

#[derive(Deserialize, Serialize)]
pub struct Event {
    pub id: isize,
    pub is_recurse: bool,
    pub outs: Vec<isize>,
    pub source: EventSource,
    pub actions: Vec<Action>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum EventSource {
    RootInit,
    RootBranch {
        parent_id: isize,
        branch_sel: isize,
        cond_wire_id: Option<isize>,
        branch_count: isize,
        branch_cond: BranchCond,
    },
    SeqCycles { pred_id: isize, cycles: isize },
    SeqSend { pred_id: isize, endpoint: String, msg: String },
    SeqRecv { pred_id: isize, endpoint: String, msg: String },
    SeqSync { pred_id: isize, var_name: String },
    Later { pred1_id: isize, pred2_id: isize },
    Branch { pred_id: isize },
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum BranchCond {
    TrueFalse(String),
    MatchCases { patterns: Vec<LoweringData> },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Action {
    DebugFinish,
    DebugPrint { fmt: String, args: Vec<LoweringData> },
    RegAssign { target: LValue, value: LoweringData },
    PutShared { name: String, value: LoweringData },
    ImmediateSend { endpoint: String, msg: String, value: LoweringData },
    ImmediateRecv { endpoint: String, msg: String },
}

/// Wrapper around the value of a wire. OCaml side also has additional metadata like lifetime values (todo)
#[derive(Deserialize, Serialize)]
pub struct LoweringData {
    pub wire_id: Option<isize>,
}

/// Name of register being assigned (left side of assignment)
/// set a := 5 (a is the lvalue)
#[derive(Deserialize, Serialize)]
pub struct LValue {
    pub reg: String,
    pub offset: isize, // constant OR computed at runtime (val stored in a wire) -> offset = wire id
    pub size: isize,
    pub offset_is_const: bool
}

#[derive(Deserialize, Serialize)]
pub struct Wire {
    pub id: isize,
    pub size: isize,
    pub is_const: bool,
    pub source: WireSource,
}

#[derive(Deserialize, Serialize)]                                                                                                                                   
pub enum BinOp {                                                                                                                                                  
    #[serde(rename = "+")]  Add,
    #[serde(rename = "-")]  Sub,                                                                                                                                    
    #[serde(rename = "*")]  Mul,                                                                                                                                    
    #[serde(rename = "^")]  Xor,                                                                                                                                    
    #[serde(rename = "&")]  And,                                                                                                                                    
    #[serde(rename = "|")]  Or,                                                                                                                                   
    #[serde(rename = "&&")] LAnd,                                                                                                                                   
    #[serde(rename = "||")] LOr,                                                                                                                                  
    #[serde(rename = "<")]  Lt,                                                                                                                                     
    #[serde(rename = ">")]  Gt,                                                                                                                                     
    #[serde(rename = "<=")] Lte,                                                                                                                                    
    #[serde(rename = ">=")] Gte,                                                                                                                                    
    #[serde(rename = "<<")] Shl,                                                                                                                                    
    #[serde(rename = ">>")] Shr,
    #[serde(rename = "==")] Eq,                                                                                                                                     
    #[serde(rename = "!=")] Neq,                                                                                                                                    
    #[serde(rename = "inside")] Inside,                                                                                                                             
}    

#[derive(Deserialize, Serialize)]
pub enum UnOp {
    #[serde(rename = "-")] Neg, // 5 -> -5
    #[serde(rename = "~")] Not, // 1001 -> 0110
    #[serde(rename = "&")] AndAll, // AND all bits together -> 1 bit res
    #[serde(rename = "|")] OrAll, // OR all buts together
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum WireSource {
    /// Constant value with a fixed bit width
    Literal { value: isize, width: isize },
    /// Read the current value of a register
    RegRead { reg: String },
    /// Binary operation on two wires. Right can be a single wire id or a list (for `inside` op)
    Binary { op: BinOp, left: isize, right: serde_json::Value },
    /// Unary operation on one wire (negation, bitwise NOT, reduction AND/OR)
    Unary { op: UnOp, operand: isize },
    /// If/else chain: first case whose cond is nonzero wins, otherwise default.
    /// e.g. if(c1) v1 else if(c2) v2 else default
    Switch { cases: Vec<SwitchCase>, default: isize },
    /// Join multiple wires into one wider value. First wire is most significant.
    /// e.g. #{a, b} where a=0b11 (2-bit), b=0b01 (2-bit) => 0b1101 (4-bit)
    Concat { wires: Vec<isize> },
    /// Extract a range of bits: wire[offset+:len]. Offset can be constant or wire id.
    Slice { wire: isize, offset: serde_json::Value, len: isize },
    /// Match a value against patterns. First matching pattern's val is returned, else default.
    /// Like a switch/case or match statement.
    Cases { value: isize, cases: Vec<SwitchCase>, default: isize },
    /// Overwrite specific bit ranges in a base value. Used for struct field updates.
    /// Each entry: replace `size` bits at `offset` with wire's value.
    Update { base: isize, updates: Vec<UpdateEntry> },
    /// Read data from a channel message port (not yet simulated)
    MessagePort { endpoint: String, msg: String, index: isize },
    /// Check if a channel message has valid data (not yet simulated)
    MessageValidPort { endpoint: String, msg: String },
    /// Check if a channel message has been acknowledged (not yet simulated)
    MessageAckPort { endpoint: String, msg: String },
}

#[derive(Deserialize, Serialize)]
pub struct SwitchCase {
    pub cond: isize,
    pub val: isize,
}

#[derive(Deserialize, Serialize)]
pub struct UpdateEntry {
    pub offset: isize,
    pub size: isize,
    pub wire: isize,
}

#[derive(Deserialize, Serialize)]
pub struct RegDef {
    pub name: String,
    pub init: Option<String>, // String of the value reg is initalised with
}
