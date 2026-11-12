from pathlib import Path
import json,ctypes,math,sys,numpy as np
libm=ctypes.CDLL('libm.so.6'); fma=libm.fma;fma.argtypes=[ctypes.c_double]*3;fma.restype=ctypes.c_double
root=Path(sys.argv[1]);receipts=[]
for file in sorted(root.glob('newton-matrix-*-matrix.f64')):
 p=str(file)[:-len('-matrix.f64')];rd=lambda s:np.fromfile(p+'-'+s+'.f64',dtype='<f8');b=rd('rhs');n=len(b);A=rd('matrix').reshape(n,n);expected=rd('solution');names=Path(p+'-names.txt').read_text().splitlines();U=A.copy();factors=np.zeros_like(U);pivots=[]
 for col in range(n):
  pivot=col+int(np.argmax(abs(U[col:,col])));pivots.append(pivot)
  if pivot!=col:U[[pivot,col]]=U[[col,pivot]]
  fs=U[col+1:,col]/U[col,col];rows=np.flatnonzero(fs!=0)+col+1
  factors[col,rows]=fs[rows-col-1];U[rows,col]=0
  U[rows,col+1:]-=factors[col,rows,None]*U[col,col+1:]
 def solve(rhs):
  rhs=rhs.copy();x=np.zeros(n)
  for col,pivot in enumerate(pivots):
   rhs[pivot],rhs[col]=rhs[col],rhs[pivot]
   rows=np.flatnonzero(factors[col,col+1:]!=0)+col+1
   rhs[rows]-=factors[col,rows]*rhs[col]
  for row in range(n-1,-1,-1):
   acc=float(rhs[row])
   for col in range(row+1,n):acc-=float(U[row,col])*float(x[col])
   x[row]=acc/U[row,row]
  return x
 rows=[[(int(c),float(A[r,c])) for c in np.nonzero(A[r,:])[0]] for r in range(n)]
 def residual(x,exact=False):
  rr=[]
  for row,terms in enumerate(rows):
   s=float(b[row]);corr=0.;exact_terms=[s]
   for col,a in terms:
    prod=-a*float(x[col]);pe=fma(-a,float(x[col]),-prod);nxt=s+prod;added=nxt-s;se=(s-(nxt-added))+(prod-added);corr+=se+pe;s=nxt;exact_terms.extend((prod,pe))
   rr.append(math.fsum(exact_terms) if exact else s+corr)
  return np.array(rr)
 x=solve(b);same=np.array_equal(x.view('u8'),expected.view('u8'));item={'matrix':file.name,'original_lu_bit_identical':same,'original_max_difference':float(max(abs(x-expected))),'passes':[]}
 for iteration in range(4):
  dx=solve(residual(x));before=x.copy();x+=dx
  selected=[66,151,127];item['passes'].append({'pass':iteration+1,'rows':[{'name':names[i],'value_before':float(before[i]),'correction':float(dx[i]),'existing_tolerance':float((1e-12 if names[i].startswith('v(') else 1e-15)+1e-9*abs(before[i])),'correction_over_tolerance':float(abs(dx[i])/((1e-12 if names[i].startswith('v(') else 1e-15)+1e-9*abs(before[i])))} for i in selected]})
 correction_oracle=solve(residual(x,exact=True)); tolerances=np.array([(1e-12 if name.startswith('v(') else 1e-15)+1e-9*abs(x[i]) for i,name in enumerate(names)]); ratios=np.abs(correction_oracle)/tolerances; worst=int(np.argmax(ratios)); item['final_max_normalized_oracle_correction']={'name':names[worst],'ratio':float(ratios[worst]),'correction':float(correction_oracle[worst]),'tolerance':float(tolerances[worst])}; item['final_oracle_correction_rows']=[{'name':names[i],'correction':float(correction_oracle[i]),'tolerance':float(1e-12+1e-9*abs(x[i]))} for i in [66,151,127]];receipts.append(item)
 print(json.dumps(item,indent=2))
(root/'native-lu-refinement-replay.json').write_text(json.dumps(receipts,indent=2)+'\n')
