# Pikchr Examples

## Usage Note

~~~ pikchr toggle
box color red wid 2.6in \
    "Click on any diagram on this page" big \
    "to see the Pikchr source text" big

~~~

## How To Build Pikchr

~~~ pikchr toggle
            filewid *= 1.2
  Src:      file "pikchr.y"; move
  LemonSrc: file "lemon.c"; move
  Lempar:   file "lempar.c"; move
            arrow down from LemonSrc.s
  CC1:      oval "C-Compiler" ht 50%
            arrow " generates" ljust above
  Lemon:    oval "lemon" ht 50%
            arrow from Src chop down until even with CC1 \
              then to Lemon.nw rad 20px
            "Pikchr source " rjust "code input " rjust \
              at 2nd vertex of previous
            arrow from Lempar chop down until even with CC1 \
              then to Lemon.ne rad 20px
            " parser template" ljust " resource file" ljust \
              at 2nd vertex of previous
  PikSrc:   file "pikchr.c" with .n at lineht below Lemon.s
            arrow from Lemon to PikSrc chop
            arrow down from PikSrc.s
  CC2:      oval "C-Compiler" ht 50%
            arrow
  Out:      file "pikchr.o" "or" "pikchr.exe" wid 110%
~~~

## SQLite Architecture Diagram

Inspired by the JPG image seen at <https://www.sqlite.org/arch.html>

~~~ pikchr toggle
    lineht *= 0.4
    $margin = lineht*2.5
    scale = 0.75
    fontscale = 1.1
    charht *= 1.15
    down
In: box "Interface" wid 150% ht 75% fill white
    arrow
CP: box same "SQL Command" "Processor"
    arrow
VM: box same "Virtual Machine"
    arrow down 1.25*$margin
BT: box same "B-Tree"
    arrow
    box same "Pager"
    arrow
OS: box same "OS Interface"
    box same with .w at 1.25*$margin east of 1st box.e "Tokenizer"
    arrow
    box same "Parser"
    arrow
CG: box same ht 200% "Code" "Generator"
UT: box same as 1st box at (Tokenizer,Pager) "Utilities"
    move lineht
TC: box same "Test Code"
    arrow from CP to 1/4<Tokenizer.sw,Tokenizer.nw> chop
    arrow from 1/3<CG.nw,CG.sw> to CP chop

    box ht (In.n.y-VM.s.y)+$margin wid In.wid+$margin \
       at CP fill 0xd8ecd0 behind In
    line invis from 0.25*$margin east of last.sw up last.ht \
        "Core" italic aligned

    box ht (BT.n.y-OS.s.y)+$margin wid In.wid+$margin \
       at Pager fill 0xd0ece8 behind In
    line invis from 0.25*$margin east of last.sw up last.ht \
       "Backend" italic aligned

    box ht (Tokenizer.n.y-CG.s.y)+$margin wid In.wid+$margin \
       at 1/2<Tokenizer.n,CG.s> fill 0xe8d8d0 behind In
    line invis from 0.25*$margin west of last.se up last.ht \
       "SQL Compiler" italic aligned

    box ht (UT.n.y-TC.s.y)+$margin wid In.wid+$margin \
       at 1/2<UT,TC> fill 0xe0ecc8 behind In
    line invis from 0.25*$margin west of last.se up last.ht \
      "Accessories" italic aligned
~~~

## Syntax diagrams

~~~ pikchr toggle
$r = 0.2in
linerad = 0.75*$r
linewid = 0.25

# Start and end blocks
#
box "element" bold fit
line down 50% from last box.sw
dot rad 250% color black
X0: last.e + (0.3,0)
arrow from last dot to X0
move right 3.9in
box wid 5% ht 25% fill black
X9: last.w - (0.3,0)
arrow from X9 to last box.w


# The main rule that goes straight through from start to finish
#
box "object-definition" italic fit at 11/16 way between X0 and X9
arrow to X9
arrow from X0 to last box.w

# The LABEL: rule
#
arrow right $r from X0 then down 1.25*$r then right $r
oval " LABEL " fit
arrow 50%
oval "\":\"" fit
arrow 200%
box "position" italic fit
arrow
line right until even with X9 - ($r,0) \
  then up until even with X9 then to X9
arrow from last oval.e right $r*0.5 then up $r*0.8 right $r*0.8
line up $r*0.45 right $r*0.45 then right

# The VARIABLE = rule
#
arrow right $r from X0 then down 2.5*$r then right $r
oval " VARIABLE " fit
arrow 70%
box "assignment-operator" italic fit
arrow 70%
box "expr" italic fit
line right until even with X9 - ($r,0) \
  then up until even with X9 then to X9

# The PRINT rule
#
arrow right $r from X0 then down 3.75*$r then right $r
oval "\"print\"" fit
arrow
box "print-args" italic fit
line right until even with X9 - ($r,0) \
  then up until even with X9 then to X9
~~~

## Swimlanes

From the [Branching, Forking, Merging, and Tagging][bfmt] paper on
the Fossil website.

[bfmt]: https://fossil-scm.org/fossil/doc/trunk/www/branching.wiki

~~~ pikchr toggle
    $laneh = 0.75

    # Draw the lanes
    down
    box width 3.5in height $laneh fill 0xacc9e3
    box same fill 0xc5d8ef
    box same as first box
    box same as 2nd box
    line from 1st box.sw+(0.2,0) up until even with 1st box.n \
      "Alan" above aligned
    line from 2nd box.sw+(0.2,0) up until even with 2nd box.n \
      "Betty" above aligned
    line from 3rd box.sw+(0.2,0) up until even with 3rd box.n \
      "Charlie" above aligned
    line from 4th box.sw+(0.2,0) up until even with 4th box.n \
       "Darlene" above aligned

    # fill in content for the Alice lane
    right
A1: circle rad 0.1in at end of first line + (0.2,-0.2) \
       fill white thickness 1.5px "1" 
    arrow right 50%
    circle same "2"
    arrow right until even with first box.e - (0.65,0.0)
    ellipse "future" fit fill white height 0.2 width 0.5 thickness 1.5px
A3: circle same at A1+(0.8,-0.3) "3" fill 0xc0c0c0
    arrow from A1 to last circle chop "fork!" below aligned

    # content for the Betty lane
B1: circle same as A1 at A1-(0,$laneh) "1"
    arrow right 50%
    circle same "2"
    arrow right until even with first ellipse.w
    ellipse same "future"
B3: circle same at A3-(0,$laneh) "3"
    arrow right 50%
    circle same as A3 "4"
    arrow from B1 to 2nd last circle chop

    # content for the Charlie lane
C1: circle same as A1 at B1-(0,$laneh) "1"
    arrow 50%
    circle same "2"
    arrow right 0.8in "goes" "offline"
C5: circle same as A3 "5"
    arrow right until even with first ellipse.w \
      "back online" above "pushes 5" below "pulls 3 &amp; 4" below
    ellipse same "future"

    # content for the Darlene lane
D1: circle same as A1 at C1-(0,$laneh) "1"
    arrow 50%
    circle same "2"
    arrow right until even with C5.w
    circle same "5"
    arrow 50%
    circle same as A3 "6"
    arrow right until even with first ellipse.w
    ellipse same "future"
D3: circle same as B3 at B3-(0,2*$laneh) "3"
    arrow 50%
    circle same "4"
    arrow from D1 to D3 chop
~~~


## Graphs

Version control graph adapted from [Rebase Considered Harmful][rch].
Commentary on the Pikchr source text to the first of these two 
graphs is provided in a [separate tutorial](./teardown01.md).

[rch]: https://fossil-scm.org/fossil/doc/trunk/www/rebaseharm.md

~~~ pikchr toggle
scale = 0.8
fill = white
linewid *= 0.5
circle "C0" fit
circlerad = previous.radius
arrow
circle "C1"
arrow
circle "C2"
arrow
circle "C4"
arrow
circle "C6"
circle "C3" at dist(C2,C4) heading 30 from C2
arrow
circle "C5"
arrow from C2 to C3 chop
C3P: circle "C3'" at dist(C4,C6) heading 30 from C6
arrow right from C3P.e
C5P: circle "C5'"
arrow from C6 to C3P chop

box height C3.y-C2.y \
    width (C5P.e.x-C0.w.x)+linewid \
    with .w at 0.5*linewid west of C0.w \
    behind C0 \
    fill 0xc6e2ff thin color gray
box same width previous.e.x - C2.w.x \
    with .se at previous.ne \
    fill 0x9accfc
"trunk" below at 2nd last box.s
"feature branch" above at last box.n

circle "C0" at 3.7cm south of C0
arrow
circle "C1"
arrow
circle "C2"
arrow
circle "C4"
arrow
circle "C6"
circle "C3" at dist(C2,C4) heading 30 from C2
arrow
circle "C5"
arrow
circle "C7"
arrow from C2 to C3 chop
arrow from C6 to C7 chop

box height C3.y-C2.y \
    width (C7.e.x-C0.w.x)+1.5*C1.radius \
    with .w at 0.5*linewid west of C0.w \
    behind C0 \
    fill 0xc6e2ff thin color gray
box same width previous.e.x - C2.w.x \
    with .se at previous.ne \
    fill 0x9accfc
"trunk" below at 2nd last box.s
"feature branch" above at last box.n
~~~

## Impossible Trident

Contributed by Kees Nuyt

~~~ pikchr toggle
# Impossible trident pikchr script
# https://en.wikipedia.org/wiki/Impossible_trident
# pikchr script by Kees Nuyt, license Creative Commons BY-NC-SA 
# https://creativecommons.org/licenses/by-nc-sa/4.0/

scale = 1.0
eh = 0.5cm
ew = 0.2cm
ed = 2 * eh
er = 0.4cm
lws = 4.0cm
lwm = lws + er
lwl = lwm + er

ellipse height eh width ew
L1: line width lwl from last ellipse.n
line width lwm from last ellipse.s
LV: line height eh down

move right er down ed from last ellipse.n
ellipse height eh width ew
L3: line width lws right from last ellipse.n to LV.end then down eh right ew
line width lwm right from last ellipse.s then to LV.start

move right er down ed from last ellipse.n
ellipse height eh width ew
line width lwl right from last ellipse.n then to L1.end
line width lwl right from last ellipse.s then up eh
~~~


## PIC Examples From The [Brian W. Kernighan paper][bwk]

[bwk]: /uv/pic.pdf

-----

From page 18.

~~~ pikchr toggle
define ndblock {
  box wid boxwid/2 ht boxht/2
  down;  box same with .t at bottom of last box;   box same
}
boxht = .2; boxwid = .3; circlerad = .3; dx = 0.05
down; box; box; box; box ht 3*boxht "." "." "."
L: box; box; box invis wid 2*boxwid "hashtab:" with .e at 1st box .w
right
Start: box wid .5 with .sw at 1st box.ne + (.4,.2) "..."
N1: box wid .2 "n1";  D1: box wid .3 "d1"
N3: box wid .4 "n3";  D3: box wid .3 "d3"
box wid .4 "..."
N2: box wid .5 "n2";  D2: box wid .2 "d2"
arrow right from 2nd box
ndblock
spline -> right .2 from 3rd last box then to N1.sw + (dx,0)
spline -> right .3 from 2nd last box then to D1.sw + (dx,0)
arrow right from last box
ndblock
spline -> right .2 from 3rd last box to N2.sw-(dx,.2) to N2.sw+(dx,0)
spline -> right .3 from 2nd last box to D2.sw-(dx,.2) to D2.sw+(dx,0)
arrow right 2*linewid from L
ndblock
spline -> right .2 from 3rd last box to N3.sw + (dx,0)
spline -> right .3 from 2nd last box to D3.sw + (dx,0)
circlerad = .3
circle invis "ndblock"  at last box.e + (1.2,.2)
arrow dashed from last circle.w to 5/8<last circle.w,2nd last box> chop
box invis wid 2*boxwid "ndtable:" with .e at Start.w
~~~


-----

From page 19:  The "intermediate code" line had to
be lengthened so that the text would fit.

~~~ pikchr toggle
        arrow "source" "code"
LA:     box "lexical" "analyzer"
        arrow "tokens" above
P:      box "parser"
        arrow "intermediate" "code" wid 200%
Sem:    box "semantic" "checker"
        arrow
        arrow <-> up from top of LA
LC:     box "lexical" "corrector"
        arrow <-> up from top of P
Syn:    box "syntactic" "corrector"
        arrow up
DMP:    box "diagnostic" "message" "printer"
        arrow <-> right  from east of DMP
ST:     box "symbol" "table"
        arrow from LC.ne to DMP.sw
        arrow from Sem.nw to DMP.se
        arrow <-> from Sem.top to ST.bot
~~~

------

From page 20:  Various minor tweaks

~~~ pikchr toggle
        circle "DISK"
        arrow "character" "defns" right 150%
CPU:    box "CPU" "(16-bit mini)"
        arrow <- from top of CPU up "input " rjust
        move right from CPU.e
CRT:    "   CRT" ljust
        line from CRT - 0,0.075 up 0.15 \
                then right 0.5 \
                then right 0.5 up 0.25 \
                then down 0.5+0.15 \
                then left 0.5 up 0.25 \
                then left 0.5
        arrow from CPU.e right until even with previous.start
Paper:  CRT + 1.05,0.75
        arrow <- from Paper down 1.5
        " ...  paper" ljust at end of last arrow + 0, 0.25
        circle rad 0.05 at Paper + (-0.055, -0.25)
        circle rad 0.05 at Paper + (0.055, -0.25)
        "   rollers" ljust at Paper + (0.1, -0.25)
~~~

-----

From [forum post 1b58e4bf9f717dbe](/forumpost/1b58e4bf9f717dbe) by
Ryan Smith:

~~~ pikchr toggle
CP:   dot;

      $h = 10;
      line from CP go 0.7cm heading $h; DA1: dot;
      line         go 1cm heading $h; DA2: dot;
      line         go 1cm heading $h; DA3: dot;
      line         go 1cm heading $h; DA4: dot;
      line         go 1cm heading $h; DA5: dot;
      line         go 1cm heading $h; DA6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 0.8cm heading $h; DB1: dot;
      line         go 1cm heading $h; DB2: dot;
      line         go 1cm heading $h; DB3: dot;
      line         go 1cm heading $h; DB4: dot;
      line         go 1cm heading $h; DB5: dot;
      line         go 1cm heading $h; DB6: dot;
      line         go 1cm heading $h;

      $h = $h+22;
      line from CP go 0.9cm heading $h; DC1: dot;
      line         go 1cm heading $h; DC2: dot;
      line         go 1cm heading $h; DC3: dot;
      line         go 1cm heading $h; DC4: dot;
      line         go 1cm heading $h; DC5: dot;
      line         go 1cm heading $h; DC6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 0.9cm heading $h; DD1: dot;
      line         go 1cm heading $h; DD2: dot;
      line         go 1cm heading $h; DD3: dot;
      line         go 1cm heading $h; DD4: dot;
      line         go 1cm heading $h; DD5: dot;
      line         go 1cm heading $h; DD6: dot;
      line         go 1cm heading $h;

      $h = $h+19;
      line from CP go 1cm heading $h; DE1: dot;
      line         go 1cm heading $h; DE2: dot;
      line         go 1cm heading $h; DE3: dot;
      line         go 1cm heading $h; DE4: dot;
      line         go 1cm heading $h; DE5: dot;
      line         go 1cm heading $h; DE6: dot;
      line         go 1cm heading $h;

      $h = $h+19;
      line from CP go 1cm heading $h; DF1: dot;
      line         go 1cm heading $h; DF2: dot;
      line         go 1cm heading $h; DF3: dot;
      line         go 1cm heading $h; DF4: dot;
      line         go 1cm heading $h; DF5: dot;
      line         go 1cm heading $h; DF6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1cm heading $h; DG1: dot;
      line         go 1cm heading $h; DG2: dot;
      line         go 1cm heading $h; DG3: dot;
      line         go 1cm heading $h; DG4: dot;
      line         go 1cm heading $h; DG5: dot;
      line         go 1cm heading $h; DG6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1cm heading $h; DH1: dot;
      line         go 1.1cm heading $h; DH2: dot;
      line         go 1.1cm heading $h; DH3: dot;
      line         go 1cm heading $h; DH4: dot;
      line         go 1cm heading $h; DH5: dot;
      line         go 1cm heading $h; DH6: dot;
      line         go 1cm heading $h;

      $h = $h+24;
      line from CP go 1cm heading $h; DI1: dot;
      line         go 1cm heading $h; DI2: dot;
      line         go 1cm heading $h; DI3: dot;
      line         go 1cm heading $h; DI4: dot;
      line         go 1cm heading $h; DI5: dot;
      line         go 1cm heading $h; DI6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 0.9cm heading $h; DJ1: dot;
      line         go 1cm heading $h; DJ2: dot;
      line         go 1cm heading $h; DJ3: dot;
      line         go 1cm heading $h; DJ4: dot;
      line         go 1cm heading $h; DJ5: dot;
      line         go 1cm heading $h; DJ6: dot;
      line         go 1cm heading $h;

      $h = $h+18;
      line from CP go 1cm heading $h; DK1: dot;
      line         go 1cm heading $h; DK2: dot;
      line         go 1cm heading $h; DK3: dot;
      line         go 1cm heading $h; DK4: dot;
      line         go 1cm heading $h; DK5: dot;
      line         go 1cm heading $h; DK6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1cm heading $h; DL1: dot;
      line         go 1cm heading $h; DL2: dot;
      line         go 1cm heading $h; DL3: dot;
      line         go 1cm heading $h; DL4: dot;
      line         go 1cm heading $h; DL5: dot;
      line         go 1cm heading $h; DL6: dot;
      line         go 1cm heading $h;

      $h = $h+19;
      line from CP go 1cm heading $h; DM1: dot;
      line         go 1cm heading $h; DM2: dot;
      line         go 1cm heading $h; DM3: dot;
      line         go 1cm heading $h; DM4: dot;
      line         go 1cm heading $h; DM5: dot;
      line         go 1cm heading $h; DM6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1cm heading $h; DN1: dot;
      line         go 1cm heading $h; DN2: dot;
      line         go 1cm heading $h; DN3: dot;
      line         go 1cm heading $h; DN4: dot;
      line         go 1cm heading $h; DN5: dot;
      line         go 1cm heading $h; DN6: dot;
      line         go 1cm heading $h;

      $h = $h+16;
      line from CP go 1.1cm heading $h; DO1: dot;
      line         go 1cm heading $h; DO2: dot;
      line         go 1cm heading $h; DO3: dot;
      line         go 1cm heading $h; DO4: dot;
      line         go 1cm heading $h; DO5: dot;
      line         go 1cm heading $h; DO6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1.2cm heading $h; DP1: dot;
      line         go 1cm heading $h; DP2: dot;
      line         go 1cm heading $h; DP3: dot;
      line         go 1cm heading $h; DP4: dot;
      line         go 1cm heading $h; DP5: dot;
      line         go 1cm heading $h; DP6: dot;
      line         go 1cm heading $h;

      $h = $h+22;
      line from CP go 1.3cm heading $h; DQ1: dot;
      line         go 1cm heading $h; DQ2: dot;
      line         go 1cm heading $h; DQ3: dot;
      line         go 1cm heading $h; DQ4: dot;
      line         go 1cm heading $h; DQ5: dot;
      line         go 1cm heading $h; DQ6: dot;
      line         go 1cm heading $h;

      $h = $h+20;
      line from CP go 1.5cm heading $h; DR1: dot;
      line         go 1cm heading $h; DR2: dot;
      line         go 1cm heading $h; DR3: dot;
      line         go 1cm heading $h; DR4: dot;
      line         go 1cm heading $h; DR5: dot;
      line         go 1cm heading $h; DR6: dot;
      line         go 0.4cm heading $h;

      line from DA1 to DB1 then to DC1 then to DD1 \
               then to DE1 then to DF1 then to DG1 \
               then to DH1 then to DI1 then to DJ1 \
               then to DK1 then to DL1 then to DM1 \
               then to DN1 then to DO1 then to DP1 \
               then to DQ1 then to DR1 then to DA2 thin;

      line          to DB2 then to DC2 then to DD2 \
               then to DE2 then to DF2 then to DG2 \
               then to DH2 then to DI2 then to DJ2 \
               then to DK2 then to DL2 then to DM2 \
               then to DN2 then to DO2 then to DP2 \
               then to DQ2 then to DR2 then to DA3 thin;

      line          to DB3 then to DC3 then to DD3 \
               then to DE3 then to DF3 then to DG3 \
               then to DH3 then to DI3 then to DJ3 \
               then to DK3 then to DL3 then to DM3 \
               then to DN3 then to DO3 then to DP3 \
               then to DQ3 then to DR3 then to DA4 thin;

      line          to DB4 then to DC4 then to DD4 \
               then to DE4 then to DF4 then to DG4 \
               then to DH4 then to DI4 then to DJ4 \
               then to DK4 then to DL4 then to DM4 \
               then to DN4 then to DO4 then to DP4 \
               then to DQ4 then to DR4 then to DA5 thin;

      line          to DB5 then to DC5 then to DD5 \
               then to DE5 then to DF5 then to DG5 \
               then to DH5 then to DI5 then to DJ5 \
               then to DK5 then to DL5 then to DM5 \
               then to DN5 then to DO5 then to DP5 \
               then to DQ5 then to DR5 then to DA6 thin;

      line          to DB6 then to DC6 then to DD6 \
               then to DE6 then to DF6 then to DG6 \
               then to DH6 then to DI6 then to DJ6 \
               then to DK6 then to DL6 then to DM6 \
               then to DN6 then to DO6 then to DP6 \
               then to DQ6 then to DR6 thin;

BODY: ellipse width 0.3cm height 0.4cm fill black at 2cm heading 340 from CP;
RDT:  ellipse width 0.2cm height 0.2cm fill red at last ellipse.c;
      ellipse width 0.2cm height 0.1cm fill black at RDT.n;
      ellipse width 0.2cm height 0.1cm fill black at RDT.s;
HEAD: ellipse with .s at BODY.n width 0.14cm height 0.12cm;
      line thin thin from HEAD.nw go 0.08cm heading 155;
      line thin thin from HEAD.ne go 0.08cm heading 205;
SPIN: ellipse thin at BODY.s width 0.06cm height 0.08cm;
      line thin from SPIN.s go 1.2cm heading 142;

  $h=340; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h- 70 then go 0.2cm heading $h-150;
  $h=306; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h-108 then go 0.2cm heading $h-150;
  $h=260; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h- 70 then go 0.2cm heading $h-110;
  $h=210; line from RDT thin chop      go 0.8cm heading $h then go 0.5cm heading $h-115 then go 0.2cm heading $h-130;

  $h= 22; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h+ 52 then go 0.2cm heading $h+130;
  $h= 45; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h+ 74 then go 0.2cm heading $h+130;
  $h= 68; line from RDT thin thin chop go 0.8cm heading $h then go 0.5cm heading $h+ 70 then go 0.2cm heading $h+134;
  $h= 92; line from RDT thin chop      go 0.8cm heading $h then go 0.5cm heading $h+100 then go 0.2cm heading $h+130;

~~~
