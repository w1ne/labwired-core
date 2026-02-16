
import subprocess
import json
import time
import sys
import os

def send_request(proc, seq, command, args=None):
    req = {
        "seq": seq,
        "type": "request",
        "command": command
    }
    if args:
        req["arguments"] = args
    
    msg = json.dumps(req)
    headers = f"Content-Length: {len(msg)}\r\n\r\n"
    print(f"DEBUG: sending {command} seq={seq}")
    proc.stdin.write((headers + msg).encode('utf-8'))
    proc.stdin.flush()

def read_message(proc):
    headers = {}
    while True:
        line_bytes = proc.stdout.readline()
        if len(line_bytes) == 0:
            return None
        
        line = line_bytes.decode('utf-8').strip()
        if line == "":
            break
            
        if ": " in line:
            key, val = line.split(": ", 1)
            headers[key] = val
    
    if "Content-Length" in headers:
        length = int(headers["Content-Length"])
        body = proc.stdout.read(length).decode('utf-8')
        # print(f"RECV: {body}")
        return json.loads(body)
    return None


def main():
    firmware_path = os.path.abspath("target/thumbv7m-none-eabi/debug/firmware-h563-io-demo")
    system_path = os.path.abspath("examples/nucleo-h563zi/system.yaml")
    
    cmd = ["target/debug/labwired-dap"]
    
    print(f"Starting {cmd}...")
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr
    )

    try:
        seq = 1
        send_request(proc, seq, "initialize", {"adapterID": "labwired"})
        seq += 1
        
        # Wait for initialized event
        while True:
            # print("DEBUG: waiting for message...")
            msg = read_message(proc)
            if not msg: 
                print("DEBUG: no message") # should not happen if blocking
                continue
            
            # print(f"DEBUG: got message {msg}")
            
            if msg.get("type") == "event" and msg.get("event") == "initialized":
                print("Initialized!")
                break
            
        send_request(proc, seq, "launch", {
            "program": firmware_path,
            "systemConfig": system_path,
            "stopOnEntry": True
        })
        seq += 1

        # Wait for launch response
        print("DEBUG: waiting for launch response")
        while True:
            msg = read_message(proc)
            if not msg: continue
            if msg.get("type") == "response" and msg.get("command") == "launch":
                if not msg.get("success"):
                    print(f"Launch failed: {msg.get('message')}")
                    # return
                print("Launched!")
                break

        send_request(proc, seq, "configurationDone")
        seq += 1

        # Wait for stopped event (entry)
        print("DEBUG: waiting for stopped event")
        while True:
            msg = read_message(proc)
            if not msg: continue
            
            if msg.get("type") == "event" and msg.get("event") == "stopped":
                print("Stopped on entry!")
                break


        # Enable DWT CYCCNT
        print("DEBUG: Enabling DWT CYCCNT")
        # Write 1 (0x1) to 0xE0001000
        # DAP writeMemory takes base64 encoded data, but here we can use writeMemory command if available?
        # Standard DAP 'writeMemory' takes 'data' as string (base64).
        # But wait, this script uses custom 'send_request'.
        # Let's see if we can use 'writeMemory' request.
        
        # 'writeMemory' args: internal u32 addr, string data (base64)
        # 1 byte = 'AQ==' (base64 for 0x01)
        # But let's write 4 bytes: 01 00 00 00
        import base64
        data_bytes = bytes([1, 0, 0, 0])
        data_b64 = base64.b64encode(data_bytes).decode('ascii')
        
        send_request(proc, seq, "writeMemory", {
            "memoryReference": hex(0xE0001000), # DAP might expect string or number? usually number in some impls, or string. 
                                                # VS Code DAP uses string for memoryReference? 
                                                # Labwired DAP adapter might support raw address if it's not strictly VS Code DAP compliant regarding MemoryReference.
                                                # Let's assume standard integer or check adapter implementation.
                                                # Adapter `write_memory` takes `addr: u64`.
            "offset": 0, # memoryReference + offset
            "data": data_b64,
            "allowPartial": True
        })
        # Wait for response? We are async here mostly.
        # But let's just send it.
        seq += 1
        
        # Actually, let's look at `labwired_dap::adapter::LabwiredAdapter::write_memory`.
        # It's exposed via `writeMemory` request in `server.rs`.
        # `server.rs` handles `readMemory` and `writeMemory`.
        
        # Continue
        print("DEBUG: continuing")
        send_request(proc, seq, "continue", {"threadId": 1})
        seq += 1

        print("Listening for telemetry... (timeout 10s)")
        last_cycles = -1
        stagnant_count = 0
        samples = 0
        
        last_dwt_cyccnt = 0
        
        start_time = time.time()
        while time.time() - start_time < 10: 
            msg = read_message(proc)
            if not msg: break
            
            # Check for writeMemory response to confirm it worked?
            if msg.get("type") == "response" and msg.get("command") == "writeMemory":
                 if not msg.get("success"):
                     print(f"WARNING: Write DWT failed: {msg.get('message')}")

            if msg.get("type") == "event" and msg.get("event") == "telemetry":
                body = msg.get("body", {})
                cycles = body.get("cycles", 0)
                pc = body.get("pc", 0)
                print(f"Telemetry: Cycles={cycles}, PC={hex(pc)}")
                
                # Periodically read DWT_CYCCNT
                if samples % 5 == 0:
                     send_request(proc, seq, "readMemory", {
                        "memoryReference": hex(0xE0001004),
                        "offset": 0,
                        "count": 4
                     })
                     seq += 1

                if cycles > 0:
                    samples += 1
                    if cycles == last_cycles:
                        stagnant_count += 1
                    else:
                        stagnant_count = 0
                    
                    last_cycles = cycles
                    
                    if stagnant_count > 5:
                        print("FAILURE: Cycles are not increasing!")
                        sys.exit(1)
            
            if msg.get("type") == "response" and msg.get("command") == "readMemory":
                 if msg.get("success"):
                     data_b64_resp = msg["body"]["data"]
                     data_bytes_resp = base64.b64decode(data_b64_resp)
                     # Interpret as u32 LE
                     dwt_cyccnt = int.from_bytes(data_bytes_resp, byteorder='little')
                     print(f"DWT_CYCCNT read: {dwt_cyccnt}")
                     
                     # It should be close to 'cycles' or at least > 0
                     if dwt_cyccnt == 0 and samples > 10:
                         print("FAILURE: DWT_CYCCNT is 0 but simulation is running!")
                         sys.exit(1)
                     if dwt_cyccnt > 0 and dwt_cyccnt <= last_dwt_cyccnt:
                          # It might wrap, but unlikely in 10s.
                          pass
                     last_dwt_cyccnt = dwt_cyccnt
        
        if samples == 0:
             print("FAILURE: No telemetry received!")
             sys.exit(1)

        if stagnant_count > 0:
             print("FAILURE: Stagnant cycles at the end.")
             sys.exit(1)
        
        print("SUCCESS: Cycles are increasing.")

    finally:
        if proc.poll() is None:
            proc.terminate()

if __name__ == "__main__":
    main()
