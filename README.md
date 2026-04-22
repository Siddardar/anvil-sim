# anvil-sim

An event-driven simulator for Anvil programs. Instead of compiling to SystemVerilog and running through Verilator, anvil-sim runs the Anvil's event graph, by compiling it into a JSON IR, directly in Rust.

## Overview
Using `cache.anvil` as an example
```
                          .anvil source
                               |
                         anvil compiler
                               |
                          JSON IR (event graphs)
                               |
                         anvil-sim deserializes
                               |
              +----------------+----------------+
              |                |                |
         OS Thread 0      OS Thread 1      OS Thread 2
         (proc: cache)    (proc: FIFO_Cache) (proc: Main_Memory)
              |                |                |
        +-----------+    +-----------+    +-----------+
        | Simulator |    | Simulator |    | Simulator |
        |           |    |           |    |           |
        | +-------+ |    | +-------+ |    | +-------+ |
        | | Heap  | |    | | Heap  | |    | | Heap  | |
        | | (min) | |    | | (min) | |    | | (min) | |
        | +-------+ |    | +-------+ |    | +-------+ |
        | | Regs  | |    | | Regs  | |    | | Regs  | |
        | +-------+ |    | +-------+ |    | +-------+ |
        | |Thread 0||    | |Thread 0||    | |Thread 0||
        | |Thread 1||    | |Thread 1||    | |Thread 1||
        | |  ...   ||    | |  ...   ||    | |Thread 2||
        +-----------+    +-----------+    +-----------+
              |                |                |
              +-------+--------+-------+--------+
                      |                |
                Arc<ChannelHandler> Arc<ChannelHandler>
                   (cache_read)      (memory_read)
                      |                |
                 +---------+      +---------+
                 | Mutex   |      | Mutex   |
                 | Condvar |      | Condvar |
                 +---------+      +---------+
```

### Procs and Threads

Each `proc` in an Anvil program becomes its own OS thread with its own `Simulator` instance. A proc can contain multiple `loop {}` blocks where each loop becomes a separate logical thread. All logical threads within a proc share the same event heap and register state.

For example, this Anvil code:

```
proc Main_Memory(...) {
    reg cycle_counter : logic[4];
    
    // Thread 0
    loop { set cycle_counter := *cycle_counter + 1 }   
    
    // Thread 1
    loop { let x = recv ch.req >> ... }                 

    // Thread 2
    loop { let y = recv write_ch.req >> ... }           
}
```

Produces one `Simulator` with three logical threads, each with its own events and wires, but sharing a single event heap and register file.

### Event Heap

Each proc has a min heap of events ordered by `(cycle, event_id, thread_idx)`. The main loop:

1. Pops the lowest-cycle event from the heap
2. Fires it (executes actions, schedules successors)
3. Fires all other events at the same cycle
4. Applies pending register writes
5. Loops

Register writes are buffered during a cycle and applied at the end, so all events within the same cycle see the same register values.

### Firing an Event

When an event fires, two things happen:

1. **Execute actions**: register assignments, debug prints, channel sends/receives
2. **Schedule successors**: each successor event has a source type that determines when it fires:
   - `SeqCycles { cycles: N }` — pushed onto the heap at `current_cycle + N`
   - `RootBranch` / `Branch` — evaluated and fired immediately (same cycle)
   - `SeqSend` / `SeqRecv` — channel operations that may [park](#parking) (i.e. evaluated later)
   - `Later` — fires when both predecessors have arrived (used for `>>` sequencing with `{...}` parallel blocks)

If an event has `is_recurse = true`, the thread's root event is re-fired after all successors are scheduled. This is how `loop {}` works — the last event in the loop body recurses back to the root.

### Wire Evaluation
`eval_wire(id)` recursively resolves a wire's value by evaluating its source when needed:

- `Literal`: constant value
- `RegRead`: reads from the register file (byte-array storage, supports arbitrary widths)
- `Binary` / `Unary`: arithmetic and logic operations
- `Slice` / `Concat` / `Update`: bit manipulation
- `Switch` / `Cases`: conditional multiplexing
- `MessagePort`: reads data from a [channel](#channels) (blocks when evaluated via `ImmediateSend`/`ImmediateRecv` actions and the channel slot is empty. For `SeqRecv` successors, the event parks instead of blocking.)

## Channels

Channels are the communication mechanism between procs. Each channel has two endpoints (left and right) and carries named messages.

### Shared Channel Table

At startup, `main.rs` builds a channel table, a `HashMap<String, Arc<ChannelHandler>>`. Each `ChannelHandler` contains a `Mutex<SharedChannel>` (holding the data) and a `Condvar` (for cross-thread signaling).

Both endpoints of a channel point to the **same** `Arc<ChannelHandler>`. Spawned procs get **aliases** that map their parameter names to the actual endpoint names in the channel table. (e.g., if `cache` declares `chan cache_input -- cache_output : cache_read` and does `spawn FIFO_Cache(cache_input, ...)`, then FIFO_Cache's parameter name `ch` is aliased to `cache_input` in its channel table, both keys point to the same `Arc<ChannelHandler>`.)

### Send and Receive

Channel operations use a single-slot protocol: each message name has one `Option<isize>` slot. `SharedChannel` holds a `HashMap<String, Option<isize>>` where each key is a message name like `"req"` or `"res"`. `None` means empty/consumed, `Some(val)` means data is available. The whole struct is behind a `Mutex` inside `ChannelHandler`.

- **Send**: Writes `Some(value)` into the slot. If the slot already has data (previous value not yet consumed), the send **parks**.
- **Receive**: Reads `Some(value)` from the slot. If the slot is `None` (no data yet), the receive [parks](#parking).
- **Clear**: After a receive completes, the slot is set back to `None`, signaling the sender that it can send again.

There are two types of send and receive:
- `ImmediateSend` / `ImmediateRecv`: executed as event actions (blocking via condvar for cross-proc communication)
- `SeqSend` / `SeqRecv`: scheduled as event successors (non-blocking, uses [parking](#parking))

### Parking

When a `SeqSend` or `SeqRecv` can't complete immediately (slot full or empty), the event is **parked** — removed from the heap and stored in a `Vec<ParkedEvent>`.

The main loop calls `try_unpark()` after each event batch. This scans all parked events and checks if their channel conditions are now satisfied:
- Parked recv: is there data in the slot?
- Parked send: is the slot empty?

If ready, the event is moved back into the heap at its original cycle and will fire on the next iteration.

### The recv_cache

There's a subtle problem: when a `SeqRecv` event fires, it processes the received value and then clears the channel slot (so the sender can send again). 

But some successor events may be parked on a different channel. When those successors eventually fire, their wire evaluations may reference a `MessagePort` wire that reads from the already-cleared channel, causing a deadlock.

Example: FIFO_Cache receives an address on `ch.req`, then on a cache miss, sends a memory request on `ch_mem.req` and parks waiting for `ch_mem.res`. After the recv fires, `ch.req` is cleared. When the memory response arrives and the parked event fires, its branch conditions evaluate wires derived from the address — which came from `MessagePort ch.req` but that slot is now `None`.

The fix: when a `SeqRecv` fires, its value is saved into a `recv_cache` (keyed by `(endpoint, msg)`). Wire evaluation for `MessagePort` checks the cache first. If the value is there, it returns it immediately without touching the channel. The cache is overwritten the next time a `SeqRecv` fires on the same `(endpoint, msg)` pair (the same channel endpoint and message name. This happens when the loop iterates and it executes again with new data, replacing the old cached value.).

### Execution Flow

When an event fires, this is the full call chain:

1. `fire_event(event)` is called
2. `execute_actions()` runs the event's actions (RegAssign, DebugPrint, ImmediateSend, etc.). Actions that need computed values call `eval_wire()`, which recursively resolves wire dependencies. If a wire is a `MessagePort` and the channel slot is empty, `eval_wire` blocks on `condvar.wait()` until another proc writes data.
3. `schedule_successors()` looks at the event's **successor edges** and decides what to do with each one based on its source type:
   - `SeqCycles` → pushed onto the heap for a future cycle
   - `Branch` / `RootBranch` → evaluates a condition wire (calling `eval_wire` again) and immediately fires the matching branch
   - `SeqRecv` → checks the channel: if data is present, fires immediately; if not, parks the event (no wire evaluation happens, avoiding the `MessagePort` block)
   - `SeqSend` → evaluates the value wire to compute what to send, then checks the channel: if the slot is empty, writes the data and fires immediately; if full, parks with the pre-computed value
4. After all same-cycle events fire, `apply_pending_writes()` commits buffered register writes

The key distinction: actions block on missing channel data (via `condvar.wait` inside `eval_wire`), while SeqSend/SeqRecv successors avoid blocking by parking instead. `SeqSend` does evaluate the value wire before deciding to park — so if that wire depends on a `MessagePort`, it can still block. But the channel check itself (is the slot free?) is non-blocking.