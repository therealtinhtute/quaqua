import json,subprocess,time,sys
def h(*a):
    r=subprocess.run(["herdr",*a],capture_output=True,text=True,timeout=10)
    try: return json.loads(r.stdout)
    except: return {"_raw":r.stdout}
agents=[("wd-cc","w9:p5"),("wd-oc2","w9:p7"),("wd-omp","w9:p6")]
for n,p in agents:
    h("agent","prompt",n,"Use your question/ask-user tool to ask me exactly: Pick A or B? options A or B. If you have no such tool reply NO_ASK_TOOL.")
t0=time.time(); seen={}
while time.time()-t0<20:
    for n,p in agents:
        a=(h("agent","get",n).get("result") or {}).get("agent") or {}
        st=a.get("agent_status")
        txt=subprocess.run(["herdr","pane","read",p,"--source","visible","--lines","30"],capture_output=True,text=True,timeout=10).stdout
        k=(n,st)
        if k not in seen:
            seen[k]=True
            print(json.dumps({"t":round(time.time()-t0,2),"name":n,"status":st,"tail":[l.strip() for l in txt.strip().splitlines()[-6:]]}))
    time.sleep(1)
