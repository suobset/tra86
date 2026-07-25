#!/usr/bin/env python3
"""Bind's out-of-process LLDB driver.

Bind's Rust core cannot link liblldb on this platform (no dev libraries), so the
LLDB *programmatic* SB API is hosted here in the interpreter LLDB ships with, and
Bind talks to it over a line-delimited JSON protocol on stdin/stdout. This is a
structured adapter over the SB API -- it never scrapes the human-readable `lldb`
console. Each request is one JSON object on a line; each response is one JSON
object on a line with the matching `id`.

Request:  {"id": <int>, "op": "<name>", ...args}
Response: {"id": <int>, "ok": true,  "result": <obj>}
          {"id": <int>, "ok": false, "error": "<msg>"}

Run with the LLDB python path on PYTHONPATH, e.g.:
    PYTHONPATH="$(lldb -P)" python3 bind_lldb_driver.py
"""
import json
import sys

try:
    import lldb
except Exception as exc:  # pragma: no cover - environment dependent
    sys.stdout.write(json.dumps({"id": 0, "ok": False,
                                 "error": "cannot import lldb module: %s" % exc}) + "\n")
    sys.stdout.flush()
    sys.exit(2)


class Driver:
    def __init__(self):
        self.dbg = lldb.SBDebugger.Create()
        self.dbg.SetAsync(False)
        self.target = None
        self.process = None

    # --- helpers -----------------------------------------------------------
    def _require_target(self):
        if not self.target or not self.target.IsValid():
            raise RuntimeError("no target loaded")
        return self.target

    def _require_process(self):
        if not self.process or not self.process.IsValid():
            raise RuntimeError("no live process")
        return self.process

    def _addr_of(self, sbaddr):
        t = self.target
        load = sbaddr.GetLoadAddress(t)
        if load != lldb.LLDB_INVALID_ADDRESS:
            return load
        return sbaddr.GetFileAddress()

    def _stop_state(self):
        proc = self.process
        if not proc or not proc.IsValid():
            return {"stopped": False}
        state = proc.GetState()
        if state == lldb.eStateExited:
            return {"exited": proc.GetExitStatus()}
        thread = proc.GetSelectedThread()
        reason = thread.GetStopReason()
        reason_str = "none"
        if reason == lldb.eStopReasonBreakpoint:
            bp_id = thread.GetStopReasonDataAtIndex(0) if thread.GetStopReasonDataCount() else 0
            reason_str = "breakpoint:%d" % bp_id
        elif reason == lldb.eStopReasonPlanComplete:
            reason_str = "step"
        elif reason == lldb.eStopReasonSignal:
            signo = thread.GetStopReasonDataAtIndex(0) if thread.GetStopReasonDataCount() else 0
            reason_str = "signal:%d" % signo
        elif reason == lldb.eStopReasonException:
            reason_str = "exception:unknown"
        frame = thread.GetFrameAtIndex(0)
        return {
            "stopped": True,
            "thread": thread.GetThreadID(),
            "reason": reason_str,
            "pc": frame.GetPC(),
        }

    def _bp_result(self, bp):
        resolved = []
        for i in range(bp.GetNumLocations()):
            loc = bp.GetLocationAtIndex(i)
            a = loc.GetAddress()
            line_entry = a.GetLineEntry()
            resolved.append({
                "address": self._addr_of(a),
                "function": a.GetFunction().GetName() or (a.GetSymbol().GetName() or None),
                "file": str(line_entry.GetFileSpec()) if line_entry.IsValid() else None,
                "line": line_entry.GetLine() if line_entry.IsValid() else None,
            })
        return {"id": bp.GetID(), "resolved": resolved}

    # --- ops ---------------------------------------------------------------
    def op_open_target(self, req):
        program = req["program"]
        self.target = self.dbg.CreateTarget(program)
        if not self.target or not self.target.IsValid():
            raise RuntimeError("could not create target for %s" % program)
        return {"triple": self.target.GetTriple()}

    def op_launch(self, req):
        t = self._require_target()
        info = lldb.SBLaunchInfo(req.get("args", []))
        if req.get("cwd"):
            info.SetWorkingDirectory(req["cwd"])
        env = req.get("env") or []
        if env:
            info.SetEnvironmentEntries(["%s=%s" % (k, v) for k, v in env], True)
        flags = info.GetLaunchFlags()
        if req.get("stop_at_entry"):
            flags |= lldb.eLaunchFlagStopAtEntry
        info.SetLaunchFlags(flags)
        err = lldb.SBError()
        self.process = t.Launch(info, err)
        if err.Fail():
            raise RuntimeError("launch failed: %s" % err.GetCString())
        return self._stop_state()

    def op_attach(self, req):
        t = self._require_target()
        err = lldb.SBError()
        info = lldb.SBAttachInfo(int(req["pid"]))
        self.process = t.Attach(info, err)
        if err.Fail():
            raise RuntimeError("attach failed: %s" % err.GetCString())
        return self._stop_state()

    def op_resume(self, req):
        proc = self._require_process()
        err = proc.Continue()
        if err.Fail():
            raise RuntimeError("continue failed: %s" % err.GetCString())
        return self._stop_state()

    def op_step(self, req):
        proc = self._require_process()
        thread = proc.GetSelectedThread()
        kind = req.get("kind", "instruction")
        if kind == "instruction":
            thread.StepInstruction(False)
        elif kind == "over":
            thread.StepOver()
        elif kind == "into":
            thread.StepInto()
        elif kind == "out":
            thread.StepOut()
        else:
            raise RuntimeError("unknown step kind: %s" % kind)
        return self._stop_state()

    def op_add_breakpoint(self, req):
        t = self._require_target()
        kind = req["kind"]
        if kind == "symbol":
            bp = t.BreakpointCreateByName(req["name"])
        elif kind == "address":
            bp = t.BreakpointCreateByAddress(int(req["address"]))
        elif kind == "source":
            bp = t.BreakpointCreateByLocation(req["file"], int(req["line"]))
        else:
            raise RuntimeError("unknown breakpoint kind: %s" % kind)
        if req.get("condition"):
            bp.SetCondition(req["condition"])
        return self._bp_result(bp)

    def op_remove_breakpoint(self, req):
        t = self._require_target()
        ok = t.BreakpointDelete(int(req["id"]))
        return {"removed": bool(ok)}

    def op_threads(self, req):
        proc = self._require_process()
        out = []
        for thread in proc:
            frame = thread.GetFrameAtIndex(0)
            out.append({
                "id": thread.GetThreadID(),
                "name": thread.GetName(),
                "pc": frame.GetPC() if frame.IsValid() else None,
            })
        return out

    def op_frames(self, req):
        proc = self._require_process()
        tid = req.get("thread")
        thread = proc.GetThreadByID(tid) if tid else proc.GetSelectedThread()
        out = []
        for i in range(thread.GetNumFrames()):
            f = thread.GetFrameAtIndex(i)
            le = f.GetLineEntry()
            out.append({
                "index": i,
                "pc": f.GetPC(),
                "sp": f.GetSP(),
                "fp": f.GetFP(),
                "function": f.GetFunctionName(),
                "module": str(f.GetModule().GetFileSpec().GetFilename())
                          if f.GetModule().IsValid() else None,
                "file": str(le.GetFileSpec()) if le.IsValid() else None,
                "line": le.GetLine() if le.IsValid() else None,
                "inlined": f.IsInlined(),
            })
        return out

    def op_registers(self, req):
        proc = self._require_process()
        tid = req.get("thread")
        thread = proc.GetThreadByID(tid) if tid else proc.GetSelectedThread()
        frame = thread.GetFrameAtIndex(0)
        out = []
        for rset in frame.GetRegisters():
            for r in rset:
                out.append({
                    "name": r.GetName(),
                    "value": r.GetValueAsUnsigned(),
                    "size": r.GetByteSize() * 8,
                })
        return out

    def op_disassemble(self, req):
        t = self._require_target()
        count = int(req.get("count", 16))
        if req.get("address") is not None:
            addr = t.ResolveLoadAddress(int(req["address"]))
            if not addr.IsValid():
                addr = lldb.SBAddress(int(req["address"]), t)
            insns = t.ReadInstructions(addr, count)
        elif self.process and self.process.IsValid():
            frame = self.process.GetSelectedThread().GetFrameAtIndex(0)
            insns = t.ReadInstructions(frame.GetPCAddress(), count)
        else:
            raise RuntimeError("no address and no running process to disassemble")
        out = []
        for i in range(insns.GetSize()):
            ins = insns.GetInstructionAtIndex(i)
            a = ins.GetAddress()
            data = ins.GetData(t)
            err = lldb.SBError()
            size = ins.GetByteSize()
            raw = data.ReadRawData(err, 0, size) if size else b""
            out.append({
                "address": self._addr_of(a),
                "bytes": raw.hex() if not err.Fail() and raw else "",
                "mnemonic": ins.GetMnemonic(t) or "",
                "operands": ins.GetOperands(t) or "",
                "function": a.GetFunction().GetName() or (a.GetSymbol().GetName() or None),
            })
        return out

    def op_read_memory(self, req):
        proc = self._require_process()
        err = lldb.SBError()
        data = proc.ReadMemory(int(req["address"]), int(req["length"]), err)
        if err.Fail():
            raise RuntimeError("read memory failed: %s" % err.GetCString())
        return {"hex": data.hex()}

    def op_modules(self, req):
        t = self._require_target()
        out = []
        for m in t.module_iter():
            out.append({
                "path": str(m.GetFileSpec().GetFilename()),
                "has_debug": m.GetNumCompileUnits() > 0,
            })
        return out

    def op_process_info(self, req):
        t = self.target
        arch = t.GetTriple() if t and t.IsValid() else None
        if self.process and self.process.IsValid():
            state = self.process.GetState()
            state_name = {
                lldb.eStateStopped: "stopped",
                lldb.eStateRunning: "running",
                lldb.eStateExited: "exited",
                lldb.eStateCrashed: "crashed",
            }.get(state, "unknown")
            return {"pid": self.process.GetProcessID(), "state": state_name, "arch": arch}
        return {"pid": None, "state": "idle", "arch": arch}

    def op_detach(self, req):
        if self.process and self.process.IsValid():
            self.process.Detach()
        return {"detached": True}

    def op_kill(self, req):
        if self.process and self.process.IsValid():
            self.process.Kill()
        return {"killed": True}

    def op_lldb_version(self, req):
        return {"version": self.dbg.GetVersionString()}


def main():
    driver = Driver()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception as exc:
            sys.stdout.write(json.dumps({"id": 0, "ok": False,
                                         "error": "bad json: %s" % exc}) + "\n")
            sys.stdout.flush()
            continue
        rid = req.get("id", 0)
        op = req.get("op", "")
        if op == "shutdown":
            sys.stdout.write(json.dumps({"id": rid, "ok": True, "result": {}}) + "\n")
            sys.stdout.flush()
            break
        handler = getattr(driver, "op_" + op, None)
        if handler is None:
            resp = {"id": rid, "ok": False, "error": "unknown op: %s" % op}
        else:
            try:
                resp = {"id": rid, "ok": True, "result": handler(req)}
            except Exception as exc:  # convert every failure into a typed error
                resp = {"id": rid, "ok": False, "error": str(exc)}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
