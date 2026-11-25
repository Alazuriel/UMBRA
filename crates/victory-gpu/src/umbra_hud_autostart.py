"""External, read-only Windows performance HUD. No injection or game memory access."""
import argparse
import collections
import ctypes as C
from ctypes import wintypes as W
import datetime as dt
import json
import math
import os
from pathlib import Path
import re
import subprocess
import threading
import time
import tkinter as tk

GIB = 1024 ** 3
FPS_RE = re.compile(r'(\d+(?:\.\d+)?) fps, (\d+(?:\.\d+)?) ms/frame.*?\((\d+) being made\)')
TITLE_RE = re.compile(r'\b(?:VIctory|UMBRA(?: build)?)\s+(\d+(?:\.\d+)?) fps\s+(\d+(?:\.\d+)?) ms/frame', re.I)
ENGINE_RE = re.compile(r'^pid_(\d+)_(luid_.+?_phys_\d+_eng_\d+)_engtype_.+$', re.I)
PS = r'''
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
function Rows($class, $props) {
  try { @(Get-CimInstance -ClassName $class -ErrorAction Stop | Select-Object -Property $props) }
  catch { $null }
}
while ($true) {
  $cpu = Rows 'Win32_PerfFormattedData_PerfOS_Processor' @('Name','PercentProcessorTime')
  $mem = Rows 'Win32_PerfFormattedData_PerfOS_Memory' @('AvailableBytes','CommittedBytes','CommitLimit')
  $eng = Rows 'Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine' @('Name','UtilizationPercentage')
  $gpu = Rows 'Win32_PerfFormattedData_GPUPerformanceCounters_GPUAdapterMemory' @('Name','DedicatedUsage','SharedUsage')
  $gp = Rows 'Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory' @('Name','DedicatedUsage','SharedUsage')
  $proc = @(Get-Process -Name victory,umbra -ErrorAction SilentlyContinue | Select-Object Id,PrivateMemorySize64,WorkingSet64)
  try { $ram = (Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize * 1024 } catch { $ram = $null }
  [Console]::WriteLine((@{cpu=@($cpu | Where-Object Name -eq '_Total');mem=@($mem);engine=@($eng);gpu=@($gpu);gp=@($gp);proc=@($proc);ram=$ram} | ConvertTo-Json -Depth 4 -Compress))
  Start-Sleep -Milliseconds 1000
}
'''

def number(v):
    try:
        n = float(v)
        return n if math.isfinite(n) and n >= 0 else None
    except (ValueError, TypeError):
        return None

def rows(d, k):
    return [r for r in (d.get(k) or []) if isinstance(r, dict)]

def summed(rs, key):
    vals = [n for r in rs if (n := number(r.get(key))) is not None]
    return sum(vals) if vals else None

def gpu_usage(rs, pid=None):
    engines = collections.defaultdict(float)
    valid = False
    for r in rs:
        m = ENGINE_RE.match(str(r.get('Name','')))
        v = number(r.get('UtilizationPercentage'))
        if not m or v is None:
            continue
        valid = True
        if pid is None or int(m[1]) == pid:
            engines[m[2].lower()] += v
    return min(100, max(engines.values(), default=0)) if valid else None

def pct(v): return '--' if v is None else f'{v:3.0f}%'
def gib(v): return '--' if v is None else f'{v/GIB:.2f} GiB'

def tail_metrics(path):
    try:
        with path.open('rb') as f:
            f.seek(0,2); f.seek(max(0,f.tell()-262144)); s=f.read().decode('utf-8','replace')
    except OSError:
        return None
    for line in reversed(s.splitlines()):
        m=FPS_RE.search(line)
        if m:
            try:
                stamp=dt.datetime.fromisoformat(line.split()[0].replace('Z','+00:00'))
                age=(dt.datetime.now(dt.timezone.utc)-stamp).total_seconds()
                if not -5 <= age <= 15: return None
            except (ValueError, TypeError): return None
            return float(m[1]),float(m[2]),int(m[3])
    return None

class Sampler:
    def __init__(self):
        self.data={}; self.when=0; self.error='Warming up'; self.stop=threading.Event(); self.proc=None
        self.thread=threading.Thread(target=self.run,daemon=True); self.thread.start()
    def run(self):
        try:
            self.proc=subprocess.Popen(['powershell.exe','-NoProfile','-NonInteractive','-Command',PS],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,encoding='utf-8',errors='replace',creationflags=subprocess.CREATE_NO_WINDOW)
            if self.stop.is_set(): self.proc.terminate(); return
            for line in self.proc.stdout:
                if self.stop.is_set(): break
                try:
                    data=json.loads(line)
                    if isinstance(data,dict): self.data=data; self.when=time.monotonic(); self.error=''
                except ValueError: continue
            self.error='Counters unavailable'
        except OSError as e: self.error=str(e)
        finally:
            if self.proc and self.proc.poll() is None: self.proc.terminate()
    def close(self):
        self.stop.set()
        if self.proc and self.proc.poll() is None: self.proc.terminate()

class WindowAPI:
    def __init__(self):
        self.u=C.WinDLL('user32',use_last_error=True); self.k=C.WinDLL('kernel32',use_last_error=True)
        self.cb=C.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM)
        def bind(d,n,ret,args):
            f=getattr(d,n); f.restype=ret; f.argtypes=args; return f
        u,k=self.u,self.k
        self.enum=bind(u,'EnumWindows',W.BOOL,[self.cb,W.LPARAM]); self.visible=bind(u,'IsWindowVisible',W.BOOL,[W.HWND]); self.iconic=bind(u,'IsIconic',W.BOOL,[W.HWND]); self.foreground=bind(u,'GetForegroundWindow',W.HWND,[])
        self.pid=bind(u,'GetWindowThreadProcessId',W.DWORD,[W.HWND,C.POINTER(W.DWORD)]); self.title=bind(u,'GetWindowTextW',C.c_int,[W.HWND,W.LPWSTR,C.c_int]); self.rect=bind(u,'GetClientRect',W.BOOL,[W.HWND,C.POINTER(W.RECT)]); self.screen=bind(u,'ClientToScreen',W.BOOL,[W.HWND,C.POINTER(W.POINT)])
        self.ancestor=bind(u,'GetAncestor',W.HWND,[W.HWND,W.UINT]); self.getlong=bind(u,'GetWindowLongPtrW',C.c_ssize_t,[W.HWND,C.c_int]); self.setlong=bind(u,'SetWindowLongPtrW',C.c_ssize_t,[W.HWND,C.c_int,C.c_ssize_t]); self.pos=bind(u,'SetWindowPos',W.BOOL,[W.HWND,W.HWND,C.c_int,C.c_int,C.c_int,C.c_int,W.UINT]); self.show=bind(u,'ShowWindow',W.BOOL,[W.HWND,C.c_int]); self.open=bind(k,'OpenProcess',W.HANDLE,[W.DWORD,W.BOOL,W.DWORD]); self.close=bind(k,'CloseHandle',W.BOOL,[W.HANDLE]); self.image=bind(k,'QueryFullProcessImageNameW',W.BOOL,[W.HANDLE,W.DWORD,W.LPWSTR,C.POINTER(W.DWORD)]); self.wait=bind(k,'WaitForSingleObject',W.DWORD,[W.HANDLE,W.DWORD])
    def getpid(self,h):
        p=W.DWORD(); self.pid(h,C.byref(p)); return p.value
    def gettitle(self,h):
        b=C.create_unicode_buffer(1024); self.title(h,b,len(b)); return b.value
    def exe(self,pid):
        h=self.open(0x1000,False,pid)
        if not h: return ''
        try:
            b=C.create_unicode_buffer(32768); n=W.DWORD(len(b)); return Path(b.value).name.lower() if self.image(h,0,b,C.byref(n)) else ''
        finally: self.close(h)
    def bounds(self,h):
        r=W.RECT(); p=W.POINT(0,0)
        if not self.rect(h,C.byref(r)) or not self.screen(h,C.byref(p)): return None
        return p.x,p.y,r.right-r.left,r.bottom-r.top
    def target(self,requested=None):
        h=self.foreground()
        if not h or not self.visible(h) or self.iconic(h): return None
        pid=self.getpid(h)
        if requested is not None:
            if pid!=requested: return None
        elif self.exe(pid) not in ('victory.exe','umbra.exe'): return None
        b=self.bounds(h)
        if not b or b[2]<320 or b[3]<180: return None
        return h,pid,b,self.gettitle(h)
    def overlay_style(self,h):
        # layered + transparent hit testing + tool window + no activation
        self.setlong(h,-20,(self.getlong(h,-20)|0x80000|0x20|0x80|0x08000000)&~0x40000)
        self.pos(h,W.HWND(-1),0,0,0,0,0x0001|0x0002|0x0010|0x0020)

class HUD:
    def __init__(self,args):
        self.args=args; self.api=WindowAPI(); self.parent_handle=None
        if args.exit_with_parent:
            if args.pid is None: raise SystemExit('--exit-with-parent requires --pid')
            self.parent_handle=self.api.open(0x00100000,False,args.pid)
            if not self.parent_handle: raise SystemExit('Runtime process unavailable')
        self.root=tk.Tk(); r=self.root
        r.withdraw(); r.overrideredirect(True); r.configure(bg='#10151e'); r.attributes('-alpha',0.92); r.attributes('-topmost',True)
        self.width=350; self.height=235
        r.geometry(f'{self.width}x{self.height}+0+0')
        self.c=tk.Canvas(r,width=self.width,height=self.height,bg='#10151e',highlightthickness=0); self.c.pack(fill='both',expand=True)
        r.update_idletasks(); r.deiconify(); r.update_idletasks()
        self.hwnd=self.api.ancestor(r.winfo_id(),2) or r.winfo_id(); self.api.overlay_style(self.hwnd); self.api.show(self.hwnd,0)
        self.shown=False; self.last_render=0; self.last_pos=None; self.sampler=Sampler(); self.last_title=None; self.last_title_time=0
        self.root.protocol('WM_DELETE_WINDOW',self.close); self.root.after(100,self.tick)
    def close(self): self.sampler.close(); self.root.destroy()
    def draw(self,pid,title):
        c=self.c; c.delete('all'); d=self.sampler.data if time.monotonic()-self.sampler.when<8 else {}
        cpu=next((number(r.get('PercentProcessorTime')) for r in rows(d,'cpu')),None)
        m=next(iter(rows(d,'mem')),{}); committed=number(m.get('CommittedBytes')); limit=number(m.get('CommitLimit')); avail=number(m.get('AvailableBytes')); ram=number(d.get('ram'))
        used=max(0,ram-avail) if ram is not None and avail is not None else None
        gpu=gpu_usage(rows(d,'engine')); gp=gpu_usage(rows(d,'engine'),pid)
        vram=summed(rows(d,'gpu'),'DedicatedUsage'); shared=summed(rows(d,'gpu'),'SharedUsage')
        p=next((r for r in rows(d,'proc') if number(r.get('Id'))==pid),{})
        pv=summed([r for r in rows(d,'gp') if str(r.get('Name','')).lower().startswith(f'pid_{pid}_')],'DedicatedUsage')
        log=tail_metrics(self.args.log) if self.args.log else None
        title_match=TITLE_RE.search(title)
        if title!=self.last_title: self.last_title=title; self.last_title_time=time.monotonic()
        # Equal title text can recur with identical measurements, so prefer fresh log.
        fps,ms,pending=log if log else (None,None,None)
        if fps is None and title_match: fps,ms=map(float,title_match.groups())
        pso=re.search(r'\bPSO (\d+)\b',title)
        if pending is None and pso: pending=int(pso[1])
        def txt(x,y,s,color='#dce6f5',size=10,bold=False,anchor='w'):
            c.create_text(x,y,text=s,fill=color,font=('Consolas',size,'bold' if bold else 'normal'),anchor=anchor)
        c.create_rectangle(0,0,self.width-1,self.height-1,outline='#35445a'); c.create_rectangle(0,0,3,self.height,fill='#a78bfa',outline='')
        txt(13,15,'UMBRA BUILD', '#c4b5fd',13,True); txt(337,15,'HUD 0.2 AUTO','#8492a9',9,anchor='e')
        c.create_line(12,30,338,30,fill='#35445a')
        txt(13,48,(' -- FPS' if fps is None else f'{fps:5.1f} FPS'),'#f5f3ff',17,True); txt(337,48,'-- ms' if ms is None else f'{ms:.1f} ms','#a8b6cb',11,anchor='e')
        def bar(y,label,val,display):
            color='#f87171' if val is not None and val>=90 else '#fbbf24' if val is not None and val>=75 else '#a78bfa'
            txt(13,y,label,'#b6c4d8'); txt(337,y,display,color,10,True,'e')
            c.create_rectangle(13,y+8,337,y+12,fill='#263245',outline='')
            if val is not None: c.create_rectangle(13,y+8,13+324*max(0,min(100,val))/100,y+12,fill=color,outline='')
        bar(75,'GPU / busiest engine',gpu,pct(gpu))
        bar(99,'CPU / system',cpu,pct(cpu))
        bar(123,'RAM / system',100*used/ram if used is not None and ram else None,f'{gib(used)} / {gib(ram)}')
        bar(147,'COMMIT',100*committed/limit if committed is not None and limit else None,f'{gib(committed)} / {gib(limit)}')
        txt(13,172,f'VRAM {gib(vram)} | shared {gib(shared)}','#dce6f5',9)
        txt(13,188,f'Runtime GPU {pct(gp)} | VRAM {gib(pv)}','#dce6f5',9)
        txt(13,204,f'Private {gib(number(p.get("PrivateMemorySize64")))} | PSO {pending if pending is not None else "--"}','#dce6f5',9)
        headroom=limit-committed if limit is not None and committed is not None else None
        status='LOW COMMIT HEADROOM' if headroom is not None and headroom<2*GIB else ('COUNTERS WARMING / N/A' if not d else 'READ-ONLY  /  WINDOWED HUD')
        txt(13,221,status,'#f87171' if status.startswith('LOW') else '#8492a9',8)
    def tick(self):
        if self.parent_handle and self.api.wait(self.parent_handle,0)!=258:
            self.close(); return
        t=self.api.target(self.args.pid)
        if t:
            _,pid,(x,y,w,h),title=t
            xx=x+12 if self.args.corner=='left' else x+max(0,w-self.width-12); yy=y+12
            if not self.shown or self.last_pos!=(xx,yy):
                self.api.pos(self.hwnd,W.HWND(-1),xx,yy,self.width,self.height,0x0010|0x0040); self.last_pos=(xx,yy); self.shown=True
            if time.monotonic()-self.last_render>=0.5:
                self.draw(pid,title); self.last_render=time.monotonic()
        elif self.shown: self.api.show(self.hwnd,0); self.shown=False
        self.root.after(150,self.tick)
    def run(self):
        try: self.root.mainloop()
        finally:
            self.sampler.close()
            if self.parent_handle: self.api.close(self.parent_handle); self.parent_handle=None

def selftest():
    a=[{'Name':'pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D','UtilizationPercentage':35},{'Name':'pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D','UtilizationPercentage':40},{'Name':'pid_1_luid_0x0_0x1_phys_0_eng_1_engtype_Copy','UtilizationPercentage':20}]
    assert gpu_usage(a)==75 and gpu_usage(a,1)==35 and gpu_usage(a,99)==0 and gpu_usage([]) is None
    assert FPS_RE.search('30.0 fps, 33.3 ms/frame (x), dropped 0 without a host pipeline (123 being made) and 0 indirect').groups()==('30.0','33.3','123')
    assert TITLE_RE.search('Test | UMBRA build 30 fps 33.3 ms/frame').groups()==('30','33.3')
    print('PASS: GPU grouping, FPS parsing, title parsing')

def main():
    p=argparse.ArgumentParser(); p.add_argument('--log',type=Path); p.add_argument('--pid',type=int); p.add_argument('--corner',choices=['left','right'],default='left'); p.add_argument('--selftest',action='store_true'); p.add_argument('--exit-with-parent',action='store_true'); a=p.parse_args()
    if a.selftest: selftest(); return
    if os.name!='nt': p.error('Windows only')
    try: C.WinDLL('user32').SetProcessDpiAwarenessContext(C.c_void_p(-4))
    except (AttributeError,OSError): pass
    HUD(a).run()
if __name__=='__main__': main()
