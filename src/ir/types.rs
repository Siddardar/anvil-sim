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
    DebugPrint { fmt: String, args: Vec<LoweringData> },
    DebugFinish,
    RegAssign { target: LValue, value: LoweringData },
    PutShared { name: String, value: LoweringData },
    ImmediateSend { endpoint: String, msg: String, value: LoweringData },
    ImmediateRecv { endpoint: String, msg: String },
}

#[derive(Deserialize, Serialize)]
pub struct LoweringData {
    pub wire_id: Option<isize>,
}

#[derive(Deserialize, Serialize)]
pub struct LValue {
    pub reg: String,
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
#[serde(tag = "type")]
pub enum WireSource {
    Literal { value: isize, width: isize },
    RegRead { reg: String },
    Binary { op: BinOp, left: isize, right: serde_json::Value },
    Unary { op: String, operand: isize },
    Switch { cases: Vec<SwitchCase>, default: isize },
    Concat { wires: Vec<isize> },
    Slice { wire: isize, offset: serde_json::Value, len: isize },
    Cases { value: isize, cases: Vec<SwitchCase>, default: isize },
    Update { base: isize, updates: Vec<UpdateEntry> },
    MessagePort { endpoint: String, msg: String, index: isize },
    MessageValidPort { endpoint: String, msg: String },
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
