#!/usr/bin/env python3
"""Draw print-sized figures for the revised manuscript.

Quality values come from the published pitch means in paper/results.md.
Screenshot PNGs are unchanged captures; only their layout and labels change.
Diagrams are constructed from explicit solid intervals and sample positions.
Requires matplotlib, numpy, and Inkscape (for the screenshot mosaics).
"""
import argparse
import base64
import html
from pathlib import Path
import re
import subprocess

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Rectangle
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
FIGURES = ROOT / "paper" / "figures"
WIDTH = 372 / 72.27  # JCGT text width in inches.
COLORS = ["#007f86", "#3972af", "#b47700", "#bc4242", "#8156ad", "#20683c"]
NAMES = ["Ray", "Voxel", "Sliced", "Scattered", "Painted", "Mesh"]
plt.rcParams.update({
    "font.family": "DejaVu Sans", "font.size": 9,
    "axes.titlesize": 9, "axes.labelsize": 9,
    "xtick.labelsize": 8, "ytick.labelsize": 8,
    "legend.fontsize": 8, "svg.fonttype": "none",
    "svg.hashsalt": "vange-rs-paper",
    "pdf.fonttype": 42, "savefig.facecolor": "white",
})

def save(fig, name):
    for suffix in ("svg", "pdf"):
        metadata = ({"Date": None} if suffix == "svg"
                    else {"CreationDate": None, "ModDate": None})
        temporary = FIGURES / f"{name}.tmp.{suffix}"
        fig.savefig(temporary, metadata=metadata)
        if suffix == "svg":
            temporary.write_text("\n".join(
                line.rstrip() for line in temporary.read_text().splitlines()) + "\n")
        temporary.replace(FIGURES / f"{name}.{suffix}")
    plt.close(fig)

def encoding():
    fig, ax = plt.subplots(figsize=(WIDTH, 2.15))
    fig.subplots_adjust(left=.02, right=.99, bottom=.03, top=.97)
    ax.set(xlim=(0, 10), ylim=(0, 4.25))
    ax.axis("off")
    for x, title in [(0.35, "Single level"), (5.35, "Dual level")]:
        ax.text(x+1.1, 3.95, title, ha="center", weight="bold")
        ax.add_patch(Rectangle((x,.35),2.2,3.2,fc="#f1f5f7",ec="#b9c2c7"))
        ax.add_patch(Rectangle((x,.35),2.2,.95,fc="#dfc49a",ec="#796146"))
        ax.text(x+1.1,.77,"Ground",ha="center")
        ax.plot([x+2.2,x+2.45],[1.3,1.3],c="#444",lw=.8)
        ax.text(x+2.53,1.3,"low",va="center",family="monospace")
    ax.text(1.45,2.6,"Air",ha="center")
    ax.add_patch(Rectangle((5.35,2.35),2.2,.7,fc="#afcbdc",ec="#496d83"))
    ax.text(6.45,2.7,"Upper slab",ha="center",va="center")
    ax.text(6.45,1.82,"Cave / air",ha="center",va="center")
    for y,label in [(2.35,"mid"),(3.05,"high")]:
        ax.plot([7.55,7.8],[y,y],c="#444",lw=.8)
        ax.text(7.88,y,label,va="center",family="monospace")
    save(fig,"encoding")

def terrain(ax):
    ax.set(xlim=(0,10),ylim=(0,4.65))
    ax.axis("off")
    ax.add_patch(Rectangle((.2,.25),9.5,.55,fc="#dfc49a",ec="#796146",lw=.8))
    ax.add_patch(Rectangle((5,1.9),3.3,.9,fc="#afcbdc",ec="#496d83",lw=.8))
    ax.text(8.55,.98,"Floor",fontsize=8,va="center")
    ax.text(6.65,2.38,"Slab",ha="center",va="center",fontsize=8)

def ray_panel(ax):
    terrain(ax)
    y=lambda x:4-(x-.5)*3/11
    xs=np.array([.5,1.7,2.9,4.1,5.3])
    ax.plot(xs,y(xs),color=COLORS[0],lw=1.6)
    ax.scatter(xs[:-1],y(xs[:-1]),s=23,facecolors="white",edgecolors=COLORS[0],zorder=4)
    ax.scatter([xs[-1]],[y(xs[-1])],s=23,color=COLORS[0],zorder=4)
    ax.plot([4.1,5.3],y(np.array([4.1,5.3])),color=COLORS[3],lw=2.7)
    ax.scatter([5],[y(5)],marker="x",s=34,color=COLORS[3],zorder=5)
    ax.annotate("Fixed forward steps",xy=(2.9,y(2.9)),xytext=(.4,1.6),
                arrowprops={"arrowstyle":"-","lw":.8},fontsize=9)
    ax.annotate("Refine this bracket",xy=(4.7,y(4.7)),xytext=(5.35,4.05),
                arrowprops={"arrowstyle":"-","lw":.8},fontsize=9)
    ax.annotate("Surface crossing",xy=(5,y(5)),xytext=(3.0,1.05),
                arrowprops={"arrowstyle":"-","lw":.8},fontsize=8)

def slice_panel(ax):
    terrain(ax)
    ys=np.arange(.5,4.1,.55)
    for y in ys:
        ax.plot([.2,9.7],[y,y],c="#aab2b7",lw=.7,ls=(0,(3,3)),zorder=1)
        if .25 <= y <= .8:
            ax.plot([.2,9.7],[y,y],c=COLORS[5],lw=2,zorder=3)
        if 1.9 <= y <= 2.8:
            ax.plot([5,8.3],[y,y],c=COLORS[5],lw=2,zorder=3)
    ax.text(.45,3.95,"Discard fragments in air",fontsize=9,
            bbox={"facecolor":"white","edgecolor":"none","pad":1})
    ax.annotate("Keep fragments inside solids",xy=(7.7,2.15),xytext=(.45,1.35),
                arrowprops={"arrowstyle":"-","lw":.8},fontsize=9,
                bbox={"facecolor":"white","edgecolor":"none","pad":1})

def sampling():
    fig,axes=plt.subplots(2,1,figsize=(WIDTH,3.65))
    fig.subplots_adjust(left=.01,right=.99,bottom=.025,top=.95,hspace=.27)
    for ax,title,draw in zip(axes,["(a) Ray marching","(b) Horizontal slices"],[ray_panel,slice_panel]):
        draw(ax)
        ax.set_title(title,loc="left",weight="bold",pad=3)
    save(fig,"sampling")
    # Keep the standalone sources consistent for readers following old links.
    for name,draw in [("ray",ray_panel),("slice",slice_panel)]:
        fig,ax=plt.subplots(figsize=(WIDTH,1.7))
        fig.subplots_adjust(left=.01,right=.99,bottom=.025,top=.98)
        draw(ax);save(fig,name)

def quality():
    text=(ROOT/"paper/results.md").read_text()
    section=text.split("Arithmetic mean over the views at each pitch, see-through / speckle (%):",1)[1]
    rows=[]
    for line in section.splitlines():
        if re.match(r"\| -?\d+° \|",line):
            cells=[s.strip() for s in line.strip("|").split("|")]
            rows.append([[float(v.strip()) for v in cell.split("/")] for cell in cells[1:]])
        if len(rows)==4:break
    values=np.array(rows)
    if values.shape!=(4,6,2):
        raise ValueError("Expected four pitches, six methods, two quality measures")
    fig,axes=plt.subplots(1,2,figsize=(WIDTH,3.15))
    fig.subplots_adjust(left=.085,right=.98,top=.88,bottom=.3,wspace=.36)
    styles=["-","--","-.",":","--","-"]
    markers=["o","s","^","v","D","x"]
    for metric,ax in enumerate(axes):
        for i,name in enumerate(NAMES):
            ax.plot([0,30,60,90],np.maximum(.05,values[:,i,metric]),
                    label=name,c=COLORS[i],ls=styles[i],marker=markers[i],lw=1.2,ms=3.5)
        ax.set_yscale("log")
        ax.set_ylim(.045,110 if metric==0 else 10)
        ax.set_xticks([0,30,60,90],["0","30","60","90"])
        ticks=[.1,1,10,100] if metric==0 else [.1,1,10]
        ax.set_yticks(ticks,[f"{v:g}" for v in ticks])
        ax.minorticks_off()
        ax.set_xlabel("Downward pitch (degrees)")
        ax.set_title(["Missing terrain (%)","Local incoherence (%)"][metric],loc="left",pad=9)
        ax.grid(axis="y",c="#d9dde1",lw=.6)
        for edge in ["top","right"]:ax.spines[edge].set_visible(False)
    fig.legend(*axes[0].get_legend_handles_labels(),loc="lower center",ncol=3,
               bbox_to_anchor=(.5,.015),frameon=False,columnspacing=1.25,handlelength=2.2)
    save(fig,"quality-pitch")

def mosaic(name):
    width=744
    if name=="teaser":
        height=356
        names=NAMES
        cells=[(i%3,i//3,i) for i in range(6)]
        x0=0;cellw=240;cellh=145;stepx=252;stepy=182;top=29
    else:
        height=394
        names=["Ray","Voxel","Mesh"]
        cells=[(col,row,col*2+row) for col in range(3) for row in range(2)]
        x0=0;cellw=240;cellh=151;stepx=252;stepy=177;top=36
    svg=[f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
         '<style>text{font-family:DejaVu Sans,sans-serif;font-size:19px;fill:#20252b}</style>',
         f'<rect width="{width}" height="{height}" fill="white"/>']
    for col,row,index in cells:
        x=x0+col*stepx;y=top+row*stepy
        title=names[index] if name=="teaser" else names[col]
        if name=="teaser" or row==0:
            svg.append(f'<text x="{x+cellw/2}" y="{y-9}" text-anchor="middle">{html.escape(title)}</text>')
        # Complete source captures are kept; no annotation is drawn over terrain.
        path=f"source/{name}-{index}.png"
        if not (FIGURES/path).is_file():
            raise FileNotFoundError(FIGURES/path)
        encoded = base64.b64encode((FIGURES/path).read_bytes()).decode("ascii")
        svg.append(f'<image xlink:href="data:image/png;base64,{encoded}" x="{x}" y="{y}" width="{cellw}" height="{cellh}" preserveAspectRatio="xMidYMid meet"/>')
        if name=="edit":
            label="Before" if row==0 else "After"
            svg.append(f'<text x="{x+8}" y="{y+cellh+18}" style="font-size:17px">{label}</text>')
    svg.append("</svg>\n")
    out=FIGURES/f"{name}.svg";out.write_text("\n".join(svg))
    temporary=out.with_suffix('.tmp.pdf')
    subprocess.run(["inkscape",str(out),"--export-type=pdf",
                    f"--export-filename={temporary}"],check=True)
    temporary.replace(out.with_suffix('.pdf'))

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only",choices=["encoding","sampling","quality","teaser","edit"])
    args=parser.parse_args()
    for name,draw in [("encoding",encoding),("sampling",sampling),("quality",quality),
                       ("teaser",lambda:mosaic("teaser")),("edit",lambda:mosaic("edit"))]:
        if args.only is None or args.only==name:draw()
    print("Rebuilt manuscript figures from diagrams, recorded means, and unchanged captures.")

if __name__=="__main__":main()
