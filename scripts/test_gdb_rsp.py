import socket
import json
import time
import subprocess

def calculate_checksum(packet):
    return sum(packet.encode()) % 256

def send_packet(s, packet):
    full_packet = f"${packet}#{calculate_checksum(packet):02x}"
    # print(f"SEND: {full_packet}")
    s.sendall(full_packet.encode())
    ack = s.recv(1)
    if ack != b'+':
        raise Exception(f"Expected ACK (+), got {ack}")

def receive_packet(s):
    data = b""
    while True:
        char = s.recv(1)
        if char == b'$':
            break
    
    packet_data = b""
    while True:
        char = s.recv(1)
        if char == b'#':
            break
        packet_data += char
    
    checksum = s.recv(2)
    s.sendall(b"+") # Send ACK
    # print(f"RECV: ${packet_data.decode()}#{checksum.decode()}")
    return packet_data.decode()

def test_gdb():
    print("🚀 Starting GDB RSP Verification...")
    
    # 1. Start DAP Server (which starts GDB server)
    # We assume it's already built.
    dap_process = subprocess.Popen(
        ["/home/andrii/Projects/labwired/target/debug/labwired-dap"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    # Initialize DAP session so machine is loaded
    init_req = json.dumps({"command": "initialize", "arguments": {"adapterID": "labwired"}, "seq": 1, "type": "request"})
    dap_process.stdin.write(f"Content-Length: {len(init_req)}\r\n\r\n{init_req}")
    dap_process.stdin.flush()
    
    # Launch with C example
    launch_req = json.dumps({
        "command": "launch", 
        "arguments": {
            "program": "/home/andrii/Projects/labwired/examples/arm-c-hello/target/firmware",
            "systemConfig": "/home/andrii/Projects/labwired/examples/arm-c-hello/system.yaml"
        }, 
        "seq": 2, 
        "type": "request"
    })
    dap_process.stdin.write(f"Content-Length: {len(launch_req)}\r\n\r\n{launch_req}")
    dap_process.stdin.flush()
    
    time.sleep(2) # Wait for startup
    
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.connect(("127.0.0.1:3333".split(':')[0], 3333))
        s.settimeout(5)
        
        print("🔗 Connected to GDB Server")
        
        # 1. qSupported
        send_packet(s, "qSupported:multiprocess+;swbreak+;hwbreak+;qRelocInsn+;fork-events+;vfork-events+;exec-events+;vContSupported+;no-resumed+")
        resp = receive_packet(s)
        print(f"📋 Supported: {resp}")
        if "PacketSize" not in resp: raise Exception("qSupported failed")

        # 2. Reading registers
        send_packet(s, "g")
        regs = receive_packet(s)
        print(f"📂 Registers (first 32 bytes): {regs[:32]}...")
        if len(regs) < 128: raise Exception("Register read failed")

        # 3. Reading memory
        send_packet(s, "m0,4")
        mem = receive_packet(s)
        print(f"🧠 Memory at 0x0: {mem}")
        if len(mem) != 8: raise Exception("Memory read failed")

        # 4. Writing memory
        send_packet(s, "M20000000,4:deadbeef")
        resp = receive_packet(s)
        print(f"✍️ Memory write at 0x20000000: {resp}")
        if resp != "OK": raise Exception("Memory write failed")
        
        send_packet(s, "m20000000,4")
        mem = receive_packet(s)
        print(f"🔍 Memory read back: {mem}")
        if mem != "deadbeef": raise Exception("Memory read back failed")

        # 5. Writing register (R0)
        # GDB expects little endian hex: 0xaabbccdd -> "ddccbbaa"
        send_packet(s, "P0=ddccbbaa") # Write R0 with 0xaabbccdd
        resp = receive_packet(s)
        print(f"✍️ Register R0 write: {resp}")
        if resp != "OK": raise Exception("Register write failed")
        
        send_packet(s, "g")
        regs = receive_packet(s)
        if not regs.startswith("ddccbbaa"):
             raise Exception(f"Register read back failed. Expected ddccbbaa but got {regs[:8]} in {regs}")
        print("🔍 Register R0 read back (success)")

        # 6. Breakpoint
        send_packet(s, "Z0,4a,2")
        resp = receive_packet(s)
        print(f"📍 Breakpoint Z0 at 0x4a: {resp}")
        if resp != "OK": raise Exception("Breakpoint set failed")

        # 7. Step (vCont)
        send_packet(s, "vCont;s:1")
        resp = receive_packet(s)
        print(f"👣 Step (vCont) result: {resp}")
        if not resp.startswith("S"): raise Exception("Step (vCont) failed")

        print("\n🎉 ALL ENHANCED GDB RSP TESTS PASSED!")
        
    except Exception as e:
        print(f"\n❌ GDB TEST FAILED: {e}")
        # exit(1) # Don't exit here so we can cleanup
    finally:
        s.close()
        dap_process.terminate()

if __name__ == "__main__":
    test_gdb()
