"""UMBRA startup screen; real progress from the same Rust runtime used to launch."""
import argparse, os, pathlib, queue, subprocess, threading, time, tkinter as tk
from tkinter import filedialog, messagebox, ttk

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SETTINGS = pathlib.Path(os.environ.get('LOCALAPPDATA',str(pathlib.Path.home())))/'Umbra'/'last-title.txt'
BG='#111119'; FG='#f2eff8'; MUTED='#a39daf'; LINE='#2b2935'; ACCENT='#bca3ff'

def parse_event(line):
    if not line.startswith('UMBRA_WARM '): return None
    p=line.strip().split(' ',2)
    if len(p)!=3: return None
    kind,body=p[1:]
    try:
        if kind=='BEGIN':
            n,w=map(int,body.split()); return kind,(n,w)
        if kind=='CACHE': return kind,body
        if kind=='SCAN':
            i,n,name=body.split(' ',2); return kind,(int(i),int(n),name)
        if kind in ('PROGRESS','END'):
            v=tuple(map(int,body.split()))
            if len(v)!=(9 if kind=='PROGRESS' else 6) or min(v)<0: return None
            return kind,v
    except (ValueError,TypeError): return None
    return None

def cache_environment(base, mode):
    if mode not in ('warm', 'lazy', 'off'): raise ValueError('Unknown cache mode')
    return dict(base, UMBRA_SHADER_CACHE='0' if mode == 'off' else '1')

def command(runtime,exe,jobs,console):
    return ([str(runtime),'umbra-warm',str(exe),'--jobs',str(jobs)], [str(runtime),'run',str(exe),'--execute','--console',console])

class App:
    def __init__(self,win,args):
        self.win=win; self.args=args; self.events=queue.Queue(); self.proc=None; self.cancelled=False; self.final=None; self.started=time.monotonic(); self.log=None
        win.title('UMBRA / Launcher'); win.configure(bg=BG); win.geometry('560x680'); win.minsize(505,620)
        win.protocol('WM_DELETE_WINDOW',self.close); win.bind('<F11>',lambda _:win.attributes('-fullscreen',not win.attributes('-fullscreen'))); win.bind('<Escape>',lambda _:win.attributes('-fullscreen',False))
        style=ttk.Style(win); style.theme_use('clam'); style.configure('Umbra.Horizontal.TProgressbar',background=ACCENT,troughcolor='#27252f',bordercolor=BG,lightcolor=ACCENT,darkcolor=ACCENT,thickness=5)
        body=tk.Frame(win,bg=BG); body.pack(fill='both',expand=True,padx=34,pady=(24,18))
        self.label(body,'UMBRA',25,FG,bold=True).pack(anchor='w')
        self.label(body,'Select how shaders are prepared before launch.',10,MUTED).pack(anchor='w',pady=(3,16))
        self.label(body,'SHADER CACHE',9,ACCENT,mono=True,bold=True).pack(anchor='w')
        self.mode=tk.StringVar(value=args.cache_mode); self.mode_buttons=[]
        modes=tk.Frame(body,bg=BG); modes.pack(fill='x',pady=(7,12))
        for value,title,desc in [('warm','Precompile','Prepare shaders before launch'),('lazy','On demand','Compile and cache during play'),('off','Disabled','Do not use disk shader cache')]:
            row=tk.Frame(modes,bg=BG); row.pack(fill='x',pady=2)
            radio=tk.Radiobutton(row,text=title,variable=self.mode,value=value,bg=BG,fg=FG,selectcolor=BG,activebackground=BG,activeforeground=ACCENT,disabledforeground=MUTED,highlightthickness=0,bd=0,font=('Segoe UI',10,'bold'),anchor='w',width=12,cursor='hand2')
            radio.pack(side='left'); self.mode_buttons.append(radio)
            self.label(row,desc,9,MUTED).pack(side='left')
        tk.Frame(body,bg=LINE,height=1).pack(fill='x',pady=(1,12))
        self.stage=self.label(body,'Awaiting initialization',13,FG,bold=True); self.stage.pack(anchor='w',fill='x')
        self.detail=self.label(body,'Select the title executable to start.',9,MUTED); self.detail.config(wraplength=430,justify='left'); self.detail.pack(anchor='w',fill='x',pady=(3,8))
        self.progress=ttk.Progressbar(body,style='Umbra.Horizontal.TProgressbar',maximum=100,mode='determinate'); self.progress.pack(fill='x',pady=(0,12))
        stats=tk.Frame(body,bg=BG); stats.pack(fill='x'); self.values=[]
        for i,name in enumerate(['HITS','TRANSLATED','INVALID / REF.','ELAPSED']):
            stats.columnconfigure(i,weight=1); cell=tk.Frame(stats,bg=BG); cell.grid(row=0,column=i,sticky='w'); self.label(cell,name,8,MUTED,mono=True).pack(anchor='w'); l=self.label(cell,'—',16,FG,mono=True); l.pack(anchor='w',pady=(2,0)); self.values.append(l)
        self.path=self.label(body,'Cache: waiting',8,MUTED,mono=True); self.path.config(wraplength=435,justify='left'); self.path.pack(anchor='w',pady=(10,1))
        self.extra=self.label(body,'D3D12 • runtime shader cache • pipeline objects compiled on demand',8,MUTED); self.extra.config(wraplength=435,justify='left'); self.extra.pack(anchor='w')
        actions=tk.Frame(body,bg=BG); actions.pack(side='bottom',fill='x',pady=(8,0))
        self.status=self.label(actions,'F11  Fullscreen',8,MUTED); self.status.config(wraplength=430,justify='left'); self.status.pack(side='bottom',anchor='w',pady=(7,0))
        self.start_btn=tk.Button(actions,text='START',command=self.start,bg=ACCENT,fg='#14101e',activebackground='#d5c3ff',activeforeground='#14101e',disabledforeground='#514764',relief='flat',bd=0,font=('Segoe UI',12,'bold'),cursor='hand2',pady=9); self.start_btn.pack(side='left',fill='x',expand=True)
        self.cancel_btn=tk.Button(actions,text='CANCEL',command=self.close,bg='#22212b',fg=FG,activebackground='#33313f',activeforeground=FG,relief='flat',bd=0,font=('Segoe UI',10,'bold'),cursor='hand2',padx=19,pady=12); self.cancel_btn.pack(side='right',padx=(9,0))
        self.win.after(100,self.poll)
    def label(self,parent,text,size,color,bold=False,mono=False):
        return tk.Label(parent,text=text,font=('Consolas' if mono else 'Segoe UI',size,'bold' if bold else 'normal'),bg=parent.cget('bg'),fg=color,anchor='w',bd=0,highlightthickness=0)
    def start(self):
        if self.proc is not None or getattr(self,'working',False): return
        if not self.args.game_exe:
            chosen=filedialog.askopenfilename(title='Select the title executable',filetypes=[('Windows executable','*.exe')]);
            if not chosen: return
            self.args.game_exe=chosen
        exe=pathlib.Path(self.args.game_exe).resolve(); runtime=pathlib.Path(self.args.runtime).resolve()
        if not runtime.is_file() or not exe.is_file(): messagebox.showerror('UMBRA','Runtime or title executable does not exist. Build the patched runtime first.'); return
        if self.mode.get() == 'warm' and not (exe.parent/'shaders.rpf').is_file(): messagebox.showerror('UMBRA','shaders.rpf was not found beside the selected executable.'); return
        cache=exe.parent/'Umbra'/'shader'; self.path.config(text='Disk cache OFF' if self.mode.get()=='off' else f'Cache: {cache}')
        try:
            SETTINGS.parent.mkdir(parents=True,exist_ok=True); SETTINGS.write_text(str(exe),encoding='utf-8')
        except OSError: pass
        self.runtime=runtime; self.exe=exe; self.started=time.monotonic(); self.working=True; self.final=None; self.cancelled=False
        self.start_btn.config(state='disabled'); self.stage.config(text='Initializing graphics cache'); self.detail.config(text='Detecting runtime D3D12 adapter...'); self.progress.config(mode='indeterminate'); self.progress.start(12)
        warm,self.play_cmd=command(runtime,exe,self.args.jobs,self.args.console)
        self.active_mode=self.mode.get()
        for radio in self.mode_buttons: radio.config(state='disabled')
        self.env=cache_environment(os.environ,self.active_mode); self.env['VICTORY_GAME_DIR']=str(exe.parent)
        # Keep all existing runtime flags and data paths; both children get the same environment.
        if self.active_mode != 'warm':
            self.progress.stop(); self.working=False
            self.final=(0,0,0,0,0,0)
            self.stage.config(text='Skipping shader precompilation')
            self.detail.config(text='Disk cache disabled' if self.mode.get()=='off' else 'Shaders compile on demand and are cached')
            self.launch()
            return
        threading.Thread(target=self.work,args=(warm,),daemon=True).start()
    def work(self,cmd):
        try:
            # Keep logs outside the source repository; Rust checks its own cache destination.
            logroot=pathlib.Path(os.environ.get('LOCALAPPDATA',str(pathlib.Path.home())))/'Umbra'/'logs'; logroot.mkdir(parents=True,exist_ok=True)
            self.log=logroot/('shader-warm-'+ROOT.name+'-'+time.strftime('%Y%m%d_%H%M%S')+'.log')
            with self.log.open('w',encoding='utf-8') as out:
                self.proc=subprocess.Popen(cmd,cwd=ROOT,env=self.env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,encoding='utf-8',errors='replace',creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
                if self.cancelled: self.proc.terminate()
                for line in self.proc.stdout:
                    out.write(line); out.flush(); event=parse_event(line)
                    if event: self.events.put(event)
                code=self.proc.wait()
            self.events.put(('EXIT',code))
        except Exception as exc: self.events.put(('ERROR',str(exc)))
    def poll(self):
        try:
            while True:
                kind,body=self.events.get_nowait()
                if kind=='BEGIN': self.extra.config(text=f'D3D12 • {body[1]} CPU workers • {body[0]} input files')
                elif kind=='SCAN':
                    i,n,name=body; self.stage.config(text=f'Discovering shaders  •  {i}/{n}'); self.detail.config(text=name); self.progress.config(mode='indeterminate'); self.progress.start(12)
                elif kind=='CACHE': self.path.config(text='Cache: '+body)
                elif kind=='PROGRESS':
                    i,n,done,total,hits,new,bad,dup,invalid=body; self.progress.stop(); self.progress.config(mode='determinate',value=(100*done/total if total else 100)); self.stage.config(text=f'Compiling shaders  •  file {i}/{n}'); self.detail.config(text=f'{done:,} / {total:,} containers in current file  •  {dup:,} duplicate records skipped'); self.values[0].config(text=f'{hits:,}'); self.values[1].config(text=f'{new:,}'); self.values[2].config(text=f'{bad+invalid:,}')
                elif kind=='END': self.final=body
                elif kind=='EXIT':
                    self.proc=None; self.working=False; self.progress.stop()
                    if self.cancelled: return
                    if body!=0 or self.final is None: self.failed(f'Warm-up did not complete (exit {body}).'); continue
                    found,hits,new,bad,dup,invalid=self.final; self.progress.config(mode='determinate',value=100); self.stage.config(text='Shader preparation complete'); self.detail.config(text=f'{hits+new:,} usable translations • {bad} refused • {invalid} invalid records');
                    if bad or invalid:
                        self.status.config(text='Some shaders are unsupported; review log.'); self.start_btn.config(text='CONTINUE TO GAME',state='normal',command=self.launch)
                    else: self.win.after(900,self.launch)
                elif kind=='ERROR': self.proc=None; self.working=False; self.failed(body)
        except queue.Empty: pass
        if getattr(self,'working',False):
            secs=int(time.monotonic()-self.started); self.values[3].config(text=f'{secs//60:02d}:{secs%60:02d}')
        self.win.after(100,self.poll)
    def failed(self,text):
        for radio in self.mode_buttons: radio.config(state='normal')
        self.progress.stop(); self.stage.config(text='Shader preparation failed'); self.detail.config(text=text); self.status.config(text=f'Log: {self.log}'); self.start_btn.config(text='RETRY',state='normal',command=self.start)
    def launch(self):
        if self.final is None or self.cancelled: return
        self.start_btn.config(state='disabled'); self.stage.config(text='Launching title...'); self.detail.config(text={'warm':'UMBRA graphics cache ready.','lazy':'On-demand shader cache.','off':'Disk shader cache disabled.'}[self.active_mode]);
        try:
            logroot=pathlib.Path(os.environ.get('LOCALAPPDATA',str(pathlib.Path.home())))/'Umbra'/'logs'; logroot.mkdir(parents=True,exist_ok=True)
            log=logroot/('game-'+ROOT.name+'-'+time.strftime('%Y%m%d_%H%M%S')+'.log')
            self.env['UMBRA_HUD_LOG']=str(log)
            with log.open('w',encoding='utf-8') as f: subprocess.Popen(self.play_cmd,cwd=ROOT,env=self.env,stdout=f,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,'CREATE_NEW_PROCESS_GROUP',0)|getattr(subprocess,'CREATE_NO_WINDOW',0))
            self.win.after(900,self.win.destroy)
        except Exception as exc: self.failed(str(exc))
    def close(self):
        if getattr(self,'working',False) and not messagebox.askyesno('UMBRA','Stop shader preparation? Completed cache entries are retained.'): return
        self.cancelled=True
        if self.proc is not None:
            try: self.proc.terminate()
            except OSError: pass
        self.win.destroy()

def main():
    p=argparse.ArgumentParser(); p.add_argument('--runtime',default=str(ROOT/'target'/'release'/'victory.exe')); p.add_argument('--game-exe'); p.add_argument('--jobs',type=int,default=min(os.cpu_count() or 4,12)); p.add_argument('--console',choices=['series-x','series-s'],default='series-x'); p.add_argument('--cache-mode',choices=['warm','lazy','off'],default='warm'); a=p.parse_args(); a.jobs=max(1,min(64,a.jobs))
    if not a.game_exe:
        try:
            last=pathlib.Path(SETTINGS.read_text(encoding='utf-8').strip())
            if last.is_file(): a.game_exe=str(last)
        except OSError: pass
    win=tk.Tk(); App(win,a); win.mainloop()
if __name__=='__main__': main()
