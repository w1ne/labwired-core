// Integration tests for step operations (step-out, step-back)
// These tests verify the core stepping behavior that step-out and step-back rely on

use labwired_core::cpu::CortexM;
use labwired_core::{Bus, Cpu, DebugControl, Machine};

/// Helper to create a simple test machine with ARM code in RAM
fn create_test_machine(code: &[u16]) -> Machine<CortexM> {
    let mut bus = labwired_core::bus::SystemBus::new();
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Write Thumb instructions to RAM starting at 0x20000000
    for (i, &instr) in code.iter().enumerate() {
        let addr = 0x2000_0000 + (i * 2) as u32;
        let bytes = [(instr & 0xFF) as u8, ((instr >> 8) & 0xFF) as u8];
        machine.write_memory(addr, &bytes).unwrap();
    }

    // Set PC to start of code
    machine.set_pc(0x2000_0000);

    machine
}

#[test]
fn test_step_modifies_pc() {
    // MOVS R0, #1  -> 0x2001
    // MOVS R1, #2  -> 0x2102
    let code = vec![0x2001, 0x2102];

    let mut machine = create_test_machine(&code);

    let initial_pc = machine.get_pc();
    assert_eq!(initial_pc, 0x2000_0000);

    // Step once
    machine.step_single().unwrap();

    // PC should have advanced by 2 (Thumb instruction size)
    let pc_after_step = machine.get_pc();
    assert_eq!(pc_after_step, 0x2000_0002, "PC should advance after step");
}

#[test]
fn test_multiple_steps() {
    // MOVS R0, #1  -> 0x2001
    // MOVS R0, #2  -> 0x2002
    // MOVS R0, #3  -> 0x2003
    let code = vec![0x2001, 0x2002, 0x2003];

    let mut machine = create_test_machine(&code);

    // Step 3 times
    machine.step_single().unwrap();
    assert_eq!(
        machine.read_core_reg(0),
        1,
        "R0 should be 1 after first step"
    );

    machine.step_single().unwrap();
    assert_eq!(
        machine.read_core_reg(0),
        2,
        "R0 should be 2 after second step"
    );

    machine.step_single().unwrap();
    assert_eq!(
        machine.read_core_reg(0),
        3,
        "R0 should be 3 after third step"
    );
}

#[test]
fn test_memory_write_and_read() {
    // Test that memory writes work correctly
    // MOVS R0, #0x20      -> 0x2014 (R0 = 0x20)
    // LSLS R0, R0, #24    -> 0x0600 (R0 = 0x20000000)
    // MOVS R1, #0x42      -> 0x2142 (R1 = 0x42)
    // STRB R1, [R0]       -> 0x7001 (Store byte R1 to [R0])
    let code = vec![0x2014, 0x0600, 0x2142, 0x7001];

    let mut machine = create_test_machine(&code);

    // Execute first 3 instructions to set up R0 and R1
    machine.step_single().unwrap(); // MOVS R0, #0x20
    machine.step_single().unwrap(); // LSLS R0, R0, #24
    machine.step_single().unwrap(); // MOVS R1, #0x42

    // Verify R0 and R1 are set correctly
    // 0x20 << 24 = 0x14000000 (not 0x20000000)
    assert_eq!(
        machine.read_core_reg(0),
        0x14000000,
        "R0 should be 0x14000000"
    );
    assert_eq!(machine.read_core_reg(1), 0x42, "R1 should be 0x42");

    // Read memory before write (using actual R0 value)
    let target_addr = machine.read_core_reg(0);
    let mem_before = machine.read_memory(target_addr, 1).unwrap();

    // Execute STRB instruction
    machine.step_single().unwrap();

    // Memory should now contain 0x42
    let mem_after = machine.read_memory(target_addr, 1).unwrap();
    assert_eq!(mem_after[0], 0x42, "Memory should contain 0x42 after STRB");
    assert_ne!(mem_before[0], mem_after[0], "Memory should have changed");
}

#[test]
fn test_stack_operations() {
    // Test PUSH and POP operations for step-out logic
    // MOVS R0, #0x20      -> 0x2014
    // LSLS R0, R0, #24    -> 0x0600 (R0 = 0x20000000)
    // ADDS R0, #0xFF      -> 0x30FF (R0 = 0x200000FF - initial SP)
    // MOV SP, R0          -> 0x4685 (SP = R0)
    // MOVS R1, #0x42      -> 0x2142
    // PUSH {R1}           -> 0xB402
    // POP {R2}            -> 0xBC04
    let code = vec![0x2014, 0x0600, 0x30FF, 0x4685, 0x2142, 0xB402, 0xBC04];

    let mut machine = create_test_machine(&code);

    // Set up SP
    machine.step_single().unwrap(); // MOVS R0, #0x20
    machine.step_single().unwrap(); // LSLS R0, R0, #24
    machine.step_single().unwrap(); // ADDS R0, #0xFF
    machine.step_single().unwrap(); // MOV SP, R0

    let sp_initial = machine.read_core_reg(13);
    // 0x20 << 24 + 0xFF = 0x140000FF
    assert_eq!(
        sp_initial, 0x140000FF,
        "SP should be initialized to 0x140000FF"
    );

    // Set R1 and push it
    machine.step_single().unwrap(); // MOVS R1, #0x42
    machine.step_single().unwrap(); // PUSH {R1}

    let sp_after_push = machine.read_core_reg(13);
    assert!(sp_after_push < sp_initial, "SP should decrease after PUSH");

    // Pop into R2
    machine.step_single().unwrap(); // POP {R2}

    let sp_after_pop = machine.read_core_reg(13);
    assert_eq!(sp_after_pop, sp_initial, "SP should be restored after POP");
    assert_eq!(
        machine.read_core_reg(2),
        0x42,
        "R2 should contain popped value"
    );
}

#[test]
fn test_function_call_stack_depth() {
    // Simulate a simple function call pattern
    // This tests the foundation for step-out logic
    // MOVS R0, #0x20      -> 0x2014
    // LSLS R0, R0, #24    -> 0x0600
    // ADDS R0, #0xFF      -> 0x30FF
    // MOV SP, R0          -> 0x4685 (Set up SP)
    // PUSH {LR}           -> 0xB500 (Function entry)
    // MOVS R0, #1         -> 0x2001 (Function body)
    // POP {PC}            -> 0xBD00 (Function return)
    let code = vec![0x2014, 0x0600, 0x30FF, 0x4685, 0xB500, 0x2001, 0xBD00];

    let mut machine = create_test_machine(&code);

    // Set up SP
    for _ in 0..4 {
        machine.step_single().unwrap();
    }

    let sp_before_call = machine.read_core_reg(13);

    // Execute PUSH {LR} (function entry)
    machine.step_single().unwrap();

    let sp_in_function = machine.read_core_reg(13);
    assert!(
        sp_in_function < sp_before_call,
        "SP should decrease when entering function (was {:#x}, now {:#x})",
        sp_before_call,
        sp_in_function
    );

    // Execute function body
    machine.step_single().unwrap();

    // Execute POP {PC} (function return)
    machine.step_single().unwrap();

    let sp_after_return = machine.read_core_reg(13);
    assert_eq!(
        sp_after_return, sp_before_call,
        "SP should be restored after function return"
    );
}
