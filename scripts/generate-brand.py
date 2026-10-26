#!/usr/bin/env python3
"""Generate fixed-column Unicode line art from geometric paths; no image assets."""
import math
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]

def mascot(scale):
    w, h = math.ceil(102*scale/2)*2, math.ceil(122*scale/4)*4
    dots = set()
    def dot(x,y):
        x,y=round(x*scale),round(y*scale)
        if 0<=x<w and 0<=y<h:dots.add((x,y))
    def line(a,b):
        n=max(1,math.ceil(math.dist(a,b)*scale*3))
        for i in range(n+1):
            t=i/n;dot(a[0]+(b[0]-a[0])*t,a[1]+(b[1]-a[1])*t)
    def path(points):
        for a,b in zip(points,points[1:]):line(a,b)
    def curve(a,b,c,d):
        pts=[]
        for i in range(121):
            t=i/120;s=1-t
            pts.append((s**3*a[0]+3*s*s*t*b[0]+3*s*t*t*c[0]+t**3*d[0],s**3*a[1]+3*s*s*t*b[1]+3*s*t*t*c[1]+t**3*d[1]))
        path(pts)
    def oval(cx,cy,rx,ry,tilt=0):
        pts=[]
        for i in range(161):
            t=2*math.pi*i/160;x=rx*math.cos(t);y=ry*math.sin(t)
            pts.append((cx+x*math.cos(tilt)-y*math.sin(tilt),cy+x*math.sin(tilt)+y*math.cos(tilt)))
        path(pts)
    # Oversized box: top plane, left plane and front, matching the seated mascot.
    path([(4,8),(71,5),(86,18),(18,21),(4,8),(4,68),(18,80),(18,21)])
    path([(18,80),(86,74),(86,18)])
    oval(39,49,7,11,.12);oval(68,47,7,11,.12)
    curve((47,63),(52,70),(60,70),(64,62))
    # Shirt, sleeves and tiny hands.
    curve((26,80),(21,86),(15,94),(13,99))
    curve((13,99),(14,104),(25,106),(28,100))
    path([(28,100),(31,92)])
    curve((36,80),(42,87),(49,88),(54,78))
    curve((60,77),(66,81),(72,86),(70,90))
    path([(70,90),(66,92),(62,85)])
    path([(27,104),(66,93),(71,97)])
    curve((14,102),(7,108),(6,110),(12,109))
    curve((12,109),(8,114),(16,114),(18,111))
    curve((18,111),(17,116),(24,115),(25,108))
    # Seated shorts and two forward oval feet; omit hidden lines.
    curve((28,108),(32,115),(38,118),(46,118))
    curve((48,115),(46,109),(48,101),(52,100))
    oval(57,111,10,9,.12)
    curve((65,99),(71,95),(77,94),(81,96))
    oval(86,102,10,12,.32)
    curve((85,92),(80,98),(79,105),(82,111))
    # Unicode braille is 2x4 dots per terminal cell, always one column wide.
    bits={(0,0):0,(0,1):1,(0,2):2,(1,0):3,(1,1):4,(1,2):5,(0,3):6,(1,3):7}
    rows=[]
    for y in range(0,h,4):
        row=''
        for x in range(0,w,2):
            mask=sum(1<<bit for (dx,dy),bit in bits.items() if (x+dx,y+dy) in dots)
            row+=chr(0x2800+mask) if mask else ' '
        rows.append(row.rstrip())
    while rows and not rows[-1]:rows.pop()
    return '\n'.join(rows)+'\n'

if __name__=='__main__':
    for name,scale in [('startup-logo-unicode.txt',.84),('startup-logo-compact.txt',.60)]:
        (ROOT/'assets'/name).write_text(mascot(scale))
