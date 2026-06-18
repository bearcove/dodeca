# The "behind" attribute

The "**behind** *object*" attribute causes the object currently under
construction to be drawn before the referenced *object*.  

Pikchr normally draws objects in the order that they appear in the
input script.  However, the "`behind`" attribute can be used to alter
the drawing order so that boxes used to implement background colors
or borders can be drawn before the objects they enclose, even though
the background-boxes are specified after the objects they enclose.

Consider this example:

~~~ pikchr toggle
    lineht *= 0.4
    $margin = lineht*2.5
    scale = 0.75
    fontscale = 1.1
    charht *= 1.15
    down
IN: box "Interface" wid 150% ht 75% fill white
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

    box ht (IN.n.y-VM.s.y)+$margin wid IN.wid+$margin \
       at CP fill 0xd8ecd0 behind IN
#                          ^^^^^^^^^
####################################
    line invis from 0.25*$margin east of last.sw up last.ht \
        "Core" italic aligned

    box ht (BT.n.y-OS.s.y)+$margin wid IN.wid+$margin \
       at Pager fill 0xd0ece8 behind IN
#                             ^^^^^^^^^
#######################################
    line invis from 0.25*$margin east of last.sw up last.ht \
       "Backend" italic aligned

    box ht (Tokenizer.n.y-CG.s.y)+$margin wid IN.wid+$margin \
       at 1/2<Tokenizer.n,CG.s> fill 0xe8d8d0 behind IN
#                                             ^^^^^^^^^
#######################################################
    line invis from 0.25*$margin west of last.se up last.ht \
       "SQL Compiler" italic aligned

    box ht (UT.n.y-TC.s.y)+$margin wid IN.wid+$margin \
       at 1/2<UT,TC> fill 0xe0ecc8 behind IN
#                                  ^^^^^^^^^
############################################
    line invis from 0.25*$margin west of last.se up last.ht \
      "Accessories" italic aligned
~~~

In the diagram above, the white
component boxes are drawn first.  Then the larger boxes that
implement the various background colors are drawn relative to
the component boxes.  The "`behind`" attribute must be used to
cause the background boxes to appear to be behind the component
boxes.  Click on the diagram to see the source text.  Comments
have been inserted into the source text to help identify the
"`behind`" attributes amid all the others.
