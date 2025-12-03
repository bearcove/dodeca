%include <VERSION.h>
%include {
/*
** Zero-Clause BSD license:
**
** Copyright (C) 2020-09-01 by D. Richard Hipp <drh@sqlite.org>
**
** Permission to use, copy, modify, and/or distribute this software for
** any purpose with or without fee is hereby granted.
**
****************************************************************************
**
** This software translates a PIC-inspired diagram language into SVG.
**
** PIKCHR (pronounced like "picture") is *mostly* backwards compatible
** with legacy PIC, though some features of legacy PIC are removed 
** (for example, the "sh" command is removed for security) and
** many enhancements are added.
**
** PIKCHR is designed for use in an internet facing web environment.
** In particular, PIKCHR is designed to safely generate benign SVG from
** source text that provided by a hostile agent. 
**
** This code was originally written by D. Richard Hipp using documentation
** from prior PIC implementations but without reference to prior code.
** All of the code in this project is original.
**
** This file implements a C-language subroutine that accepts a string
** of PIKCHR language text and generates a second string of SVG output that
** renders the drawing defined by the input.  Space to hold the returned
** string is obtained from malloc() and should be freed by the caller.
** NULL might be returned if there is a memory allocation error.
**
** If there are errors in the PIKCHR input, the output will consist of an
** error message and the original PIKCHR input text (inside of <pre>...</pre>).
**
** The subroutine implemented by this file is intended to be stand-alone.
** It uses no external routines other than routines commonly found in
** the standard C library.
**
****************************************************************************
** COMPILING:
**
** The original source text is a mixture of C99 and "Lemon"
** (See https://sqlite.org/src/file/doc/lemon.html).  Lemon is an LALR(1)
** parser generator program, similar to Yacc.  The grammar of the
** input language is specified in Lemon.  C-code is attached.  Lemon
** runs to generate a single output file ("pikchr.c") which is then
** compiled to generate the Pikchr library.  This header comment is
** preserved in the Lemon output, so you might be reading this in either
** the generated "pikchr.c" file that is output by Lemon, or in the
** "pikchr.y" source file that is input into Lemon.  If you make changes,
** you should change the input source file "pikchr.y", not the
** Lemon-generated output file.
**
** Basic compilation steps:
**
**      lemon pikchr.y
**      cc pikchr.c -o pikchr.o
**
** Add -DPIKCHR_SHELL to add a main() routine that reads input files
** and sends them through Pikchr, for testing.  Add -DPIKCHR_FUZZ for
** -fsanitizer=fuzzer testing.
**
****************************************************************************
** IMPLEMENTATION NOTES (for people who want to understand the internal
** operation of this software, perhaps to extend the code or to fix bugs):
**
** Each call to pikchr() uses a single instance of the Pik structure to
** track its internal state.  The Pik structure lives for the duration
** of the pikchr() call.
**
** The input is a sequence of objects or "statements".  Each statement is
** parsed into a PObj object.  These are stored on an extensible array
** called PList.  All parameters to each PObj are computed as the
** object is parsed.  (Hence, the parameters to a PObj may only refer
** to prior statements.) Once the PObj is completely assembled, it is
** added to the end of a PList and never changes thereafter - except,
** PObj objects that are part of a "[...]" block might have their
** absolute position shifted when the outer [...] block is positioned.
** But apart from this repositioning, PObj objects are unchanged once
** they are added to the list. The order of statements on a PList does
** not change.
**
** After all input has been parsed, the top-level PList is walked to
** generate output.  Sub-lists resulting from [...] blocks are scanned
** as they are encountered.  All input must be collected and parsed ahead
** of output generation because the size and position of statements must be
** known in order to compute a bounding box on the output.
**
** Each PObj is on a "layer".  (The common case is that all PObj's are
** on a single layer, but multiple layers are possible.)  A separate pass
** is made through the list for each layer.
**
** After all output is generated, the Pik object and all the PList
** and PObj objects are deallocated and the generated output string is
** returned.  Upon any error, the Pik.nErr flag is set, processing quickly
** stops, and the stack unwinds.  No attempt is made to continue reading
** input after an error.
**
** Most statements begin with a class name like "box" or "arrow" or "move".
** There is a class named "text" which is used for statements that begin
** with a string literal.  You can also specify the "text" class.
** A Sublist ("[...]") is a single object that contains a pointer to
** its substatements, all gathered onto a separate PList object.
**
** Variables go into PVar objects that form a linked list.
**
** Each PObj has zero or one names.  Input constructs that attempt
** to assign a new name from an older name, for example:
**
**      Abc:  Abc + (0.5cm, 0)
**
** Statements like these generate a new "noop" object at the specified
** place and with the given name. As place-names are searched by scanning
** the list in reverse order, this has the effect of overriding the "Abc"
** name when referenced by subsequent objects.
*/
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <math.h>
#include <assert.h>
#define count(X) (sizeof(X)/sizeof(X[0]))
#ifndef M_PI
# define M_PI 3.1415926535897932385
#endif

/*
** Typesafe version of ctype.h macros.  Cygwin requires this, I'm told.
*/
#define IsUpper(X)  isupper((unsigned char)(X))
#define IsLower(X)  islower((unsigned char)(X))
#define ToLower(X)  tolower((unsigned char)(X))
#define IsDigit(X)  isdigit((unsigned char)(X))
#define IsXDigit(X) isxdigit((unsigned char)(X))
#define IsSpace(X)  isspace((unsigned char)(X))
#define IsAlnum(X)  isalnum((unsigned char)(X))


/* Limit the number of tokens in a single script to avoid run-away
** macro expansion attacks.  See forum post
**    https://pikchr.org/home/forumpost/ef8684c6955a411a
*/
#ifndef PIKCHR_TOKEN_LIMIT
# define PIKCHR_TOKEN_LIMIT 100000
#endif


/* Tag intentionally unused parameters with this macro to prevent
** compiler warnings with -Wextra */
#define UNUSED_PARAMETER(X)  (void)(X)

typedef struct Pik Pik;          /* Complete parsing context */
typedef struct PToken PToken;    /* A single token */
typedef struct PObj PObj;        /* A single diagram object */
typedef struct PList PList;      /* A list of diagram objects */
typedef struct PClass PClass;    /* Description of statements types */
typedef double PNum;             /* Numeric value */
typedef struct PRel PRel;        /* Absolute or percentage value */
typedef struct PPoint PPoint;    /* A position in 2-D space */
typedef struct PVar PVar;        /* script-defined variable */
typedef struct PBox PBox;        /* A bounding box */
typedef struct PMacro PMacro;    /* A "define" macro */

/* Compass points */
#define CP_N      1
#define CP_NE     2
#define CP_E      3
#define CP_SE     4
#define CP_S      5
#define CP_SW     6
#define CP_W      7
#define CP_NW     8
#define CP_C      9   /* .center or .c */
#define CP_END   10   /* .end */
#define CP_START 11   /* .start */

/* Heading angles corresponding to compass points */
static const PNum pik_hdg_angle[] = {
/* none  */   0.0,
  /* N  */    0.0,
  /* NE */   45.0,
  /* E  */   90.0,
  /* SE */  135.0,
  /* S  */  180.0,
  /* SW */  225.0,
  /* W  */  270.0,
  /* NW */  315.0,
  /* C  */    0.0,
};

/* Built-in functions */
#define FN_ABS    0
#define FN_COS    1
#define FN_INT    2
#define FN_MAX    3
#define FN_MIN    4
#define FN_SIN    5
#define FN_SQRT   6

/* Text position and style flags.  Stored in PToken.eCode so limited
** to 15 bits. */
#define TP_LJUST   0x0001  /* left justify......          */
#define TP_RJUST   0x0002  /*            ...Right justify */
#define TP_JMASK   0x0003  /* Mask for justification bits */
#define TP_ABOVE2  0x0004  /* Position text way above PObj.ptAt */
#define TP_ABOVE   0x0008  /* Position text above PObj.ptAt */
#define TP_CENTER  0x0010  /* On the line */
#define TP_BELOW   0x0020  /* Position text below PObj.ptAt */
#define TP_BELOW2  0x0040  /* Position text way below PObj.ptAt */
#define TP_VMASK   0x007c  /* Mask for text positioning flags */
#define TP_BIG     0x0100  /* Larger font */
#define TP_SMALL   0x0200  /* Smaller font */
#define TP_XTRA    0x0400  /* Amplify TP_BIG or TP_SMALL */
#define TP_SZMASK  0x0700  /* Font size mask */
#define TP_ITALIC  0x1000  /* Italic font */
#define TP_BOLD    0x2000  /* Bold font */
#define TP_MONO    0x4000  /* Monospace font family */
#define TP_FMASK   0x7000  /* Mask for font style */
#define TP_ALIGN   0x8000  /* Rotate to align with the line */

/* An object to hold a position in 2-D space */
struct PPoint {
  PNum x, y;             /* X and Y coordinates */
};
static const PPoint cZeroPoint = {0.0,0.0};

/* A bounding box */
struct PBox {
  PPoint sw, ne;         /* Lower-left and top-right corners */
};

/* An Absolute or a relative distance.  The absolute distance
** is stored in rAbs and the relative distance is stored in rRel.
** Usually, one or the other will be 0.0.  When using a PRel to
** update an existing value, the computation is usually something
** like this:
**
**          value = PRel.rAbs + value*PRel.rRel
**
*/
struct PRel {
  PNum rAbs;            /* Absolute value */
  PNum rRel;            /* Value relative to current value */
};

/* A variable created by the ID = EXPR construct of the PIKCHR script 
**
** PIKCHR (and PIC) scripts do not use many varaibles, so it is reasonable
** to store them all on a linked list.
*/
struct PVar {
  const char *zName;       /* Name of the variable */
  PNum val;                /* Value of the variable */
  PVar *pNext;             /* Next variable in a list of them all */
};

/* A single token in the parser input stream
*/
struct PToken {
  const char *z;             /* Pointer to the token text */
  unsigned int n;            /* Length of the token in bytes */
  short int eCode;           /* Auxiliary code */
  unsigned char eType;       /* The numeric parser code */
  unsigned char eEdge;       /* Corner value for corner keywords */
};

/* Return negative, zero, or positive if pToken is less than, equal to
** or greater than the zero-terminated string z[]
*/
static int pik_token_eq(PToken *pToken, const char *z){
  int c = strncmp(pToken->z,z,pToken->n);
  if( c==0 && z[pToken->n]!=0 ) c = -1;
  return c;
}

/* Extra token types not generated by LEMON but needed by the
** tokenizer
*/
#define T_PARAMETER  253     /* $1, $2, ..., $9 */
#define T_WHITESPACE 254     /* Whitespace or comments */
#define T_ERROR      255     /* Any text that is not a valid token */

/* Directions of movement */
#define DIR_RIGHT     0
#define DIR_DOWN      1
#define DIR_LEFT      2
#define DIR_UP        3
#define ValidDir(X)     ((X)>=0 && (X)<=3)
#define IsUpDown(X)     (((X)&1)==1)
#define IsLeftRight(X)  (((X)&1)==0)

/* Bitmask for the various attributes for PObj.  These bits are
** collected in PObj.mProp and PObj.mCalc to check for constraint
** errors. */
#define A_WIDTH         0x0001
#define A_HEIGHT        0x0002
#define A_RADIUS        0x0004
#define A_THICKNESS     0x0008
#define A_DASHED        0x0010 /* Includes "dotted" */
#define A_FILL          0x0020
#define A_COLOR         0x0040
#define A_ARROW         0x0080
#define A_FROM          0x0100
#define A_CW            0x0200
#define A_AT            0x0400
#define A_TO            0x0800 /* one or more movement attributes */
#define A_FIT           0x1000


/* A single graphics object */
struct PObj {
  const PClass *type;      /* Object type or class */
  PToken errTok;           /* Reference token for error messages */
  PPoint ptAt;             /* Reference point for the object */
  PPoint ptEnter, ptExit;  /* Entry and exit points */
  PList *pSublist;         /* Substructure for [...] objects */
  char *zName;             /* Name assigned to this statement */
  PNum w;                  /* "width" property */
  PNum h;                  /* "height" property */
  PNum rad;                /* "radius" property */
  PNum sw;                 /* "thickness" property. (Mnemonic: "stroke width")*/
  PNum dotted;             /* "dotted" property.   <=0.0 for off */
  PNum dashed;             /* "dashed" property.   <=0.0 for off */
  PNum fill;               /* "fill" property.  Negative for off */
  PNum color;              /* "color" property */
  PPoint with;             /* Position constraint from WITH clause */
  char eWith;              /* Type of heading point on WITH clause */
  char cw;                 /* True for clockwise arc */
  char larrow;             /* Arrow at beginning (<- or <->) */
  char rarrow;             /* Arrow at end  (-> or <->) */
  char bClose;             /* True if "close" is seen */
  char bChop;              /* True if "chop" is seen */
  char bAltAutoFit;        /* Always send both h and w into xFit() */        
  unsigned char nTxt;      /* Number of text values */
  unsigned mProp;          /* Masks of properties set so far */
  unsigned mCalc;          /* Values computed from other constraints */
  PToken aTxt[5];          /* Text with .eCode holding TP flags */
  int iLayer;              /* Rendering order */
  int inDir, outDir;       /* Entry and exit directions */
  int nPath;               /* Number of path points */
  PPoint *aPath;           /* Array of path points */
  PObj *pFrom, *pTo;       /* End-point objects of a path */
  PBox bbox;               /* Bounding box */
};

/* A list of graphics objects */
struct PList {
  int n;          /* Number of statements in the list */
  int nAlloc;     /* Allocated slots in a[] */
  PObj **a;       /* Pointers to individual objects */
};

/* A macro definition */
struct PMacro {
  PMacro *pNext;       /* Next in the list */
  PToken macroName;    /* Name of the macro */
  PToken macroBody;    /* Body of the macro */
  int inUse;           /* Do not allow recursion */
};

/* Each call to the pikchr() subroutine uses an instance of the following
** object to pass around context to all of its subroutines.
*/
struct Pik {
  unsigned nErr;           /* Number of errors seen */
  unsigned nToken;         /* Number of tokens parsed */
  PToken sIn;              /* Input Pikchr-language text */
  char *zOut;              /* Result accumulates here */
  unsigned int nOut;       /* Bytes written to zOut[] so far */
  unsigned int nOutAlloc;  /* Space allocated to zOut[] */
  unsigned char eDir;      /* Current direction */
  unsigned int mFlags;     /* Flags passed to pikchr() */
  PObj *cur;               /* Object under construction */
  PObj *lastRef;           /* Last object references by name */
  PList *list;             /* Object list under construction */
  PMacro *pMacros;         /* List of all defined macros */
  PVar *pVar;              /* Application-defined variables */
  PBox bbox;               /* Bounding box around all statements */
                           /* Cache of layout values.  <=0.0 for unknown... */
  PNum rScale;                 /* Multiply to convert inches to pixels */
  PNum fontScale;              /* Scale fonts by this percent */
  PNum charWidth;              /* Character width */
  PNum charHeight;             /* Character height */
  PNum wArrow;                 /* Width of arrowhead at the fat end */
  PNum hArrow;                 /* Ht of arrowhead - dist from tip to fat end */
  char bLayoutVars;            /* True if cache is valid */
  char thenFlag;           /* True if "then" seen */
  char samePath;           /* aTPath copied by "same" */
  const char *zClass;      /* Class name for the <svg> */
  int wSVG, hSVG;          /* Width and height of the <svg> */
  int fgcolor;             /* foreground color value, or -1 for none */
  int bgcolor;             /* background color value, or -1 for none */
  /* Paths for lines are constructed here first, then transferred into
  ** the PObj object at the end: */
  int nTPath;              /* Number of entries on aTPath[] */
  int mTPath;              /* For last entry, 1: x set,  2: y set */
  PPoint aTPath[1000];     /* Path under construction */
  /* Error contexts */
  unsigned int nCtx;       /* Number of error contexts */
  PToken aCtx[10];         /* Nested error contexts */
};

/* Include PIKCHR_PLAINTEXT_ERRORS among the bits of mFlags on the 3rd
** argument to pikchr() in order to cause error message text to come out
** as text/plain instead of as text/html
*/
#define PIKCHR_PLAINTEXT_ERRORS 0x0001

/* Include PIKCHR_DARK_MODE among the mFlag bits to invert colors.
*/
#define PIKCHR_DARK_MODE        0x0002

/*
** The behavior of an object class is defined by an instance of
** this structure. This is the "virtual method" table.
*/
struct PClass {
  const char *zName;                     /* Name of class */
  char isLine;                           /* True if a line class */
  char eJust;                            /* Use box-style text justification */
  void (*xInit)(Pik*,PObj*);              /* Initializer */
  void (*xNumProp)(Pik*,PObj*,PToken*);   /* Value change notification */
  void (*xCheck)(Pik*,PObj*);             /* Checks to do after parsing */
  PPoint (*xChop)(Pik*,PObj*,PPoint*);    /* Chopper */
  PPoint (*xOffset)(Pik*,PObj*,int);      /* Offset from .c to edge point */
  void (*xFit)(Pik*,PObj*,PNum w,PNum h); /* Size to fit text */
  void (*xRender)(Pik*,PObj*);            /* Render */
};


/* Forward declarations */
static void pik_append(Pik*, const char*,int);
static void pik_append_text(Pik*,const char*,int,int);
static void pik_append_num(Pik*,const char*,PNum);
static void pik_append_point(Pik*,const char*,PPoint*);
static void pik_append_x(Pik*,const char*,PNum,const char*);
static void pik_append_y(Pik*,const char*,PNum,const char*);
static void pik_append_xy(Pik*,const char*,PNum,PNum);
static void pik_append_dis(Pik*,const char*,PNum,const char*);
static void pik_append_arc(Pik*,PNum,PNum,PNum,PNum);
static void pik_append_clr(Pik*,const char*,PNum,const char*,int);
static void pik_append_style(Pik*,PObj*,int);
static void pik_append_txt(Pik*,PObj*, PBox*);
static void pik_draw_arrowhead(Pik*,PPoint*pFrom,PPoint*pTo,PObj*);
static void pik_chop(PPoint*pFrom,PPoint*pTo,PNum);
static void pik_error(Pik*,PToken*,const char*);
static void pik_elist_free(Pik*,PList*);
static void pik_elem_free(Pik*,PObj*);
static void pik_render(Pik*,PList*);
static PList *pik_elist_append(Pik*,PList*,PObj*);
static PObj *pik_elem_new(Pik*,PToken*,PToken*,PList*);
static void pik_set_direction(Pik*,int);
static void pik_elem_setname(Pik*,PObj*,PToken*);
static int pik_round(PNum);
static void pik_set_var(Pik*,PToken*,PNum,PToken*);
static PNum pik_value(Pik*,const char*,int,int*);
static int pik_value_int(Pik*,const char*,int,int*);
static PNum pik_lookup_color(Pik*,PToken*);
static PNum pik_get_var(Pik*,PToken*);
static PNum pik_atof(PToken*);
static void pik_after_adding_attributes(Pik*,PObj*);
static void pik_elem_move(PObj*,PNum dx, PNum dy);
static void pik_elist_move(PList*,PNum dx, PNum dy);
static void pik_set_numprop(Pik*,PToken*,PRel*);
static void pik_set_clrprop(Pik*,PToken*,PNum);
static void pik_set_dashed(Pik*,PToken*,PNum*);
static void pik_then(Pik*,PToken*,PObj*);
static void pik_add_direction(Pik*,PToken*,PRel*);
static void pik_move_hdg(Pik*,PRel*,PToken*,PNum,PToken*,PToken*);
static void pik_evenwith(Pik*,PToken*,PPoint*);
static void pik_set_from(Pik*,PObj*,PToken*,PPoint*);
static void pik_add_to(Pik*,PObj*,PToken*,PPoint*);
static void pik_close_path(Pik*,PToken*);
static void pik_set_at(Pik*,PToken*,PPoint*,PToken*);
static short int pik_nth_value(Pik*,PToken*);
static PObj *pik_find_nth(Pik*,PObj*,PToken*);
static PObj *pik_find_byname(Pik*,PObj*,PToken*);
static PPoint pik_place_of_elem(Pik*,PObj*,PToken*);
static int pik_bbox_isempty(PBox*);
static int pik_bbox_contains_point(PBox*,PPoint*);
static void pik_bbox_init(PBox*);
static void pik_bbox_addbox(PBox*,PBox*);
static void pik_bbox_add_xy(PBox*,PNum,PNum);
static void pik_bbox_addellipse(PBox*,PNum x,PNum y,PNum rx,PNum ry);
static void pik_add_txt(Pik*,PToken*,int);
static int pik_text_length(const PToken *pToken, const int isMonospace);
static void pik_size_to_fit(Pik*,PObj*,PToken*,int);
static int pik_text_position(int,PToken*);
static PNum pik_property_of(PObj*,PToken*);
static PNum pik_func(Pik*,PToken*,PNum,PNum);
static PPoint pik_position_between(PNum x, PPoint p1, PPoint p2);
static PPoint pik_position_at_angle(PNum dist, PNum r, PPoint pt);
static PPoint pik_position_at_hdg(PNum dist, PToken *pD, PPoint pt);
static void pik_same(Pik *p, PObj*, PToken*);
static PPoint pik_nth_vertex(Pik *p, PToken *pNth, PToken *pErr, PObj *pObj);
static PToken pik_next_semantic_token(PToken *pThis);
static void pik_compute_layout_settings(Pik*);
static void pik_behind(Pik*,PObj*);
static PObj *pik_assert(Pik*,PNum,PToken*,PNum);
static PObj *pik_position_assert(Pik*,PPoint*,PToken*,PPoint*);
static PNum pik_dist(PPoint*,PPoint*);
static void pik_add_macro(Pik*,PToken *pId,PToken *pCode);


} // end %include

%name pik_parser
%token_prefix T_
%token_type {PToken}
%extra_context {Pik *p}

%fallback ID EDGEPT.

// precedence rules.
%left OF.
%left PLUS MINUS.
%left STAR SLASH PERCENT.
%right UMINUS.

%type statement_list {PList*}
%destructor statement_list {pik_elist_free(p,$$);}
%type statement {PObj*}
%destructor statement {pik_elem_free(p,$$);}
%type unnamed_statement {PObj*}
%destructor unnamed_statement {pik_elem_free(p,$$);}
%type basetype {PObj*}
%destructor basetype {pik_elem_free(p,$$);}
%type expr {PNum}
%type numproperty {PToken}
%type edge {PToken}
%type isodate {PToken}
%type direction {PToken}
%type dashproperty {PToken}
%type colorproperty {PToken}
%type locproperty {PToken}
%type position {PPoint}
%type place {PPoint}
%type object {PObj*}
%type objectname {PObj*}
%type nth {PToken}
%type textposition {short int}
%type rvalue {PNum}
%type lvalue {PToken}
%type even {PToken}
%type relexpr {PRel}
%type optrelexpr {PRel}

%syntax_error {
  if( TOKEN.z && TOKEN.z[0] ){
    pik_error(p, &TOKEN, "syntax error");
  }else{
    pik_error(p, 0, "syntax error");
  }
  UNUSED_PARAMETER(yymajor);
}
%stack_overflow {
  pik_error(p, 0, "parser stack overflow");
}

document ::= statement_list(X).  {pik_render(p,X);}


statement_list(A) ::= statement(X).   { A = pik_elist_append(p,0,X); }
statement_list(A) ::= statement_list(B) EOL statement(X).
                      { A = pik_elist_append(p,B,X); }


statement(A) ::= .   { A = 0; }
statement(A) ::= direction(D).  { pik_set_direction(p,D.eCode);  A=0; }
statement(A) ::= lvalue(N) ASSIGN(OP) rvalue(X). {pik_set_var(p,&N,X,&OP); A=0;}
statement(A) ::= PLACENAME(N) COLON unnamed_statement(X).
               { A = X;  pik_elem_setname(p,X,&N); }
statement(A) ::= PLACENAME(N) COLON position(P).
               { A = pik_elem_new(p,0,0,0);
                 if(A){ A->ptAt = P; pik_elem_setname(p,A,&N); }}
statement(A) ::= unnamed_statement(X).  {A = X;}
statement(A) ::= print prlist.  {pik_append(p,"<br>\n",5); A=0;}

// assert() statements are undocumented and are intended for testing and
// debugging use only.  If the equality comparison of the assert() fails
// then an error message is generated.
statement(A) ::= ASSERT LP expr(X) EQ(OP) expr(Y) RP. {A=pik_assert(p,X,&OP,Y);}
statement(A) ::= ASSERT LP position(X) EQ(OP) position(Y) RP.  
                                          {A=pik_position_assert(p,&X,&OP,&Y);}
statement(A) ::= DEFINE ID(ID) CODEBLOCK(C).  {A=0; pik_add_macro(p,&ID,&C);}

lvalue(A) ::= ID(A).
lvalue(A) ::= FILL(A).
lvalue(A) ::= COLOR(A).
lvalue(A) ::= THICKNESS(A).

// PLACENAME might actually be a color name (ex: DarkBlue).  But we
// cannot make it part of expr due to parsing ambiguities.  The
// rvalue non-terminal means "general expression or a colorname"
rvalue(A) ::= expr(A).
rvalue(A) ::= PLACENAME(C).  {A = pik_lookup_color(p,&C);}

print ::= PRINT.
prlist ::= pritem.
prlist ::= prlist prsep pritem.
pritem ::= FILL(X).        {pik_append_num(p,"",pik_value(p,X.z,X.n,0));}
pritem ::= COLOR(X).       {pik_append_num(p,"",pik_value(p,X.z,X.n,0));}
pritem ::= THICKNESS(X).   {pik_append_num(p,"",pik_value(p,X.z,X.n,0));}
pritem ::= rvalue(X).      {pik_append_num(p,"",X);}
pritem ::= STRING(S).      {pik_append_text(p,S.z+1,S.n-2,0);}
prsep  ::= COMMA.          {pik_append(p, " ", 1);}

%token ISODATE.

unnamed_statement(A) ::= basetype(X) attribute_list.  
                          {A = X; pik_after_adding_attributes(p,A);}

basetype(A) ::= CLASSNAME(N).            {A = pik_elem_new(p,&N,0,0); }
basetype(A) ::= STRING(N) textposition(P).
                            {N.eCode = P; A = pik_elem_new(p,0,&N,0); }
basetype(A) ::= LB savelist(L) statement_list(X) RB(E).
      { p->list = L; A = pik_elem_new(p,0,0,X); if(A) A->errTok = E; }

%type savelist {PList*}
// No destructor required as this same PList is also held by
// an "statement" non-terminal deeper on the stack.
savelist(A) ::= .   {A = p->list; p->list = 0;}

direction(A) ::= UP(A).
direction(A) ::= DOWN(A).
direction(A) ::= LEFT(A).
direction(A) ::= RIGHT(A).

relexpr(A) ::= expr(B).             {A.rAbs = B; A.rRel = 0;}
relexpr(A) ::= expr(B) PERCENT.     {A.rAbs = 0; A.rRel = B/100;}
optrelexpr(A) ::= relexpr(A).
optrelexpr(A) ::= .                 {A.rAbs = 0; A.rRel = 1.0;}

attribute_list ::= relexpr(X) alist.    {pik_add_direction(p,0,&X);}
attribute_list ::= alist.
alist ::=.
alist ::= alist attribute.
attribute ::= numproperty(P) relexpr(X).     { pik_set_numprop(p,&P,&X); }
attribute ::= dashproperty(P) expr(X).       { pik_set_dashed(p,&P,&X); }
attribute ::= dashproperty(P).               { pik_set_dashed(p,&P,0);  }
attribute ::= colorproperty(P) rvalue(X).    { pik_set_clrprop(p,&P,X); }
attribute ::= go direction(D) optrelexpr(X). { pik_add_direction(p,&D,&X);}
attribute ::= go direction(D) even position(P). {pik_evenwith(p,&D,&P);}
attribute ::= CLOSE(E).             { pik_close_path(p,&E); }
attribute ::= CHOP.                 { p->cur->bChop = 1; }
attribute ::= FROM(T) position(X).  { pik_set_from(p,p->cur,&T,&X); }
attribute ::= TO(T) position(X).    { pik_add_to(p,p->cur,&T,&X); }
attribute ::= THEN(T).              { pik_then(p, &T, p->cur); }
attribute ::= THEN(E) optrelexpr(D) HEADING(H) expr(A).
                                                {pik_move_hdg(p,&D,&H,A,0,&E);}
attribute ::= THEN(E) optrelexpr(D) EDGEPT(C).  {pik_move_hdg(p,&D,0,0,&C,&E);}
attribute ::= GO(E) optrelexpr(D) HEADING(H) expr(A).
                                                {pik_move_hdg(p,&D,&H,A,0,&E);}
attribute ::= GO(E) optrelexpr(D) EDGEPT(C).    {pik_move_hdg(p,&D,0,0,&C,&E);}
attribute ::= boolproperty.
attribute ::= AT(A) position(P).                    { pik_set_at(p,0,&P,&A); }
attribute ::= WITH withclause.
attribute ::= SAME(E).                          {pik_same(p,0,&E);}
attribute ::= SAME(E) AS object(X).             {pik_same(p,X,&E);}
attribute ::= STRING(T) textposition(P).        {pik_add_txt(p,&T,P);}
attribute ::= FIT(E).                           {pik_size_to_fit(p,0,&E,3); }
attribute ::= BEHIND object(X).                 {pik_behind(p,X);}

go ::= GO.
go ::= .

even ::= UNTIL EVEN WITH.
even ::= EVEN WITH.

withclause ::=  DOT_E edge(E) AT(A) position(P).{ pik_set_at(p,&E,&P,&A); }
withclause ::=  edge(E) AT(A) position(P).      { pik_set_at(p,&E,&P,&A); }

// Properties that require an argument
numproperty(A) ::= HEIGHT|WIDTH|RADIUS|DIAMETER|THICKNESS(P).  {A = P;}

// Properties with optional arguments
dashproperty(A) ::= DOTTED(A).
dashproperty(A) ::= DASHED(A).

// Color properties
colorproperty(A) ::= FILL(A).
colorproperty(A) ::= COLOR(A).

// Properties with no argument
boolproperty ::= CW.          {p->cur->cw = 1;}
boolproperty ::= CCW.         {p->cur->cw = 0;}
boolproperty ::= LARROW.      {p->cur->larrow=1; p->cur->rarrow=0; }
boolproperty ::= RARROW.      {p->cur->larrow=0; p->cur->rarrow=1; }
boolproperty ::= LRARROW.     {p->cur->larrow=1; p->cur->rarrow=1; }
boolproperty ::= INVIS.       {p->cur->sw = -0.00001;}
boolproperty ::= THICK.       {p->cur->sw *= 1.5;}
boolproperty ::= THIN.        {p->cur->sw *= 0.67;}
boolproperty ::= SOLID.       {p->cur->sw = pik_value(p,"thickness",9,0);
                               p->cur->dotted = p->cur->dashed = 0.0;}

textposition(A) ::= .   {A = 0;}
textposition(A) ::= textposition(B) 
   CENTER|LJUST|RJUST|ABOVE|BELOW|ITALIC|BOLD|MONO|ALIGNED|BIG|SMALL(F).
                        {A = (short int)pik_text_position(B,&F);}


position(A) ::= expr(X) COMMA expr(Y).                {A.x=X; A.y=Y;}
position(A) ::= place(A).
position(A) ::= place(B) PLUS expr(X) COMMA expr(Y).  {A.x=B.x+X; A.y=B.y+Y;}
position(A) ::= place(B) MINUS expr(X) COMMA expr(Y). {A.x=B.x-X; A.y=B.y-Y;}
position(A) ::= place(B) PLUS LP expr(X) COMMA expr(Y) RP.
                                                      {A.x=B.x+X; A.y=B.y+Y;}
position(A) ::= place(B) MINUS LP expr(X) COMMA expr(Y) RP.
                                                      {A.x=B.x-X; A.y=B.y-Y;}
position(A) ::= LP position(X) COMMA position(Y) RP.  {A.x=X.x; A.y=Y.y;}
position(A) ::= LP position(X) RP.                    {A=X;}
position(A) ::= expr(X) between position(P1) AND position(P2).
                                       {A = pik_position_between(X,P1,P2);}
position(A) ::= expr(X) LT position(P1) COMMA position(P2) GT.
                                       {A = pik_position_between(X,P1,P2);}
position(A) ::= expr(X) ABOVE position(B).    {A=B; A.y += X;}
position(A) ::= expr(X) BELOW position(B).    {A=B; A.y -= X;}
position(A) ::= expr(X) LEFT OF position(B).  {A=B; A.x -= X;}
position(A) ::= expr(X) RIGHT OF position(B). {A=B; A.x += X;}
position(A) ::= expr(D) ON HEADING EDGEPT(E) OF position(P).
                                        {A = pik_position_at_hdg(D,&E,P);}
position(A) ::= expr(D) HEADING EDGEPT(E) OF position(P).
                                        {A = pik_position_at_hdg(D,&E,P);}
position(A) ::= expr(D) EDGEPT(E) OF position(P).
                                        {A = pik_position_at_hdg(D,&E,P);}
position(A) ::= expr(D) ON HEADING expr(G) FROM position(P).
                                        {A = pik_position_at_angle(D,G,P);}
position(A) ::= expr(D) HEADING expr(G) FROM position(P).
                                        {A = pik_position_at_angle(D,G,P);}

between ::= WAY BETWEEN.
between ::= BETWEEN.
between ::= OF THE WAY BETWEEN.

// place2 is the same as place, but excludes the forms like
// "RIGHT of object" to avoid a parsing ambiguity with "place .x"
// and "place .y" expressions
%type place2 {PPoint}

place(A) ::= place2(A).
place(A) ::= edge(X) OF object(O).           {A = pik_place_of_elem(p,O,&X);}
place2(A) ::= object(O).                     {A = pik_place_of_elem(p,O,0);}
place2(A) ::= object(O) DOT_E edge(X).       {A = pik_place_of_elem(p,O,&X);}
place2(A) ::= NTH(N) VERTEX(E) OF object(X). {A = pik_nth_vertex(p,&N,&E,X);}

edge(A) ::= CENTER(A).
edge(A) ::= EDGEPT(A).
edge(A) ::= TOP(A).
edge(A) ::= BOTTOM(A).
edge(A) ::= START(A).
edge(A) ::= END(A).
edge(A) ::= RIGHT(A).
edge(A) ::= LEFT(A).

object(A) ::= objectname(A).
object(A) ::= nth(N).                     {A = pik_find_nth(p,0,&N);}
object(A) ::= nth(N) OF|IN object(B).     {A = pik_find_nth(p,B,&N);}

objectname(A) ::= THIS.                   {A = p->cur;}
objectname(A) ::= PLACENAME(N).           {A = pik_find_byname(p,0,&N);}
objectname(A) ::= objectname(B) DOT_U PLACENAME(N).
                                          {A = pik_find_byname(p,B,&N);}

nth(A) ::= NTH(N) CLASSNAME(ID).      {A=ID; A.eCode = pik_nth_value(p,&N); }
nth(A) ::= NTH(N) LAST CLASSNAME(ID). {A=ID; A.eCode = -pik_nth_value(p,&N); }
nth(A) ::= LAST CLASSNAME(ID).        {A=ID; A.eCode = -1;}
nth(A) ::= LAST(ID).                  {A=ID; A.eCode = -1;}
nth(A) ::= NTH(N) LB(ID) RB.          {A=ID; A.eCode = pik_nth_value(p,&N);}
nth(A) ::= NTH(N) LAST LB(ID) RB.     {A=ID; A.eCode = -pik_nth_value(p,&N);}
nth(A) ::= LAST LB(ID) RB.            {A=ID; A.eCode = -1; }

expr(A) ::= expr(X) PLUS expr(Y).                 {A=X+Y;}
expr(A) ::= expr(X) MINUS expr(Y).                {A=X-Y;}
expr(A) ::= expr(X) STAR expr(Y).                 {A=X*Y;}
expr(A) ::= expr(X) SLASH(E) expr(Y).             {
  if( Y==0.0 ){ pik_error(p, &E, "division by zero"); A = 0.0; }
  else{ A = X/Y; }
}
expr(A) ::= MINUS expr(X). [UMINUS]               {A=-X;}
expr(A) ::= PLUS expr(X). [UMINUS]                {A=X;}
expr(A) ::= LP expr(X) RP.                        {A=X;}
expr(A) ::= LP FILL|COLOR|THICKNESS(X) RP.        {A=pik_get_var(p,&X);}
expr(A) ::= NUMBER(N).                            {A=pik_atof(&N);}
expr(A) ::= ID(N).                                {A=pik_get_var(p,&N);}
expr(A) ::= FUNC1(F) LP expr(X) RP.               {A = pik_func(p,&F,X,0.0);}
expr(A) ::= FUNC2(F) LP expr(X) COMMA expr(Y) RP. {A = pik_func(p,&F,X,Y);}
expr(A) ::= DIST LP position(X) COMMA position(Y) RP. {A = pik_dist(&X,&Y);}
expr(A) ::= place2(B) DOT_XY X.                   {A = B.x;}
expr(A) ::= place2(B) DOT_XY Y.                   {A = B.y;}
expr(A) ::= object(B) DOT_L numproperty(P).       {A=pik_property_of(B,&P);}
expr(A) ::= object(B) DOT_L dashproperty(P).      {A=pik_property_of(B,&P);}
expr(A) ::= object(B) DOT_L colorproperty(P).     {A=pik_property_of(B,&P);}


%code {


/* Chart of the 148 official CSS color names with their
** corresponding RGB values thru Color Module Level 4:
** https://developer.mozilla.org/en-US/docs/Web/CSS/color_value
**
** Two new names "None" and "Off" are added with a value
** of -1.
*/
static const struct {
  const char *zName;  /* Name of the color */
  int val;            /* RGB value */
} aColor[] = {
  { "AliceBlue",                   0xf0f8ff },
  { "AntiqueWhite",                0xfaebd7 },
  { "Aqua",                        0x00ffff },
  { "Aquamarine",                  0x7fffd4 },
  { "Azure",                       0xf0ffff },
  { "Beige",                       0xf5f5dc },
  { "Bisque",                      0xffe4c4 },
  { "Black",                       0x000000 },
  { "BlanchedAlmond",              0xffebcd },
  { "Blue",                        0x0000ff },
  { "BlueViolet",                  0x8a2be2 },
  { "Brown",                       0xa52a2a },
  { "BurlyWood",                   0xdeb887 },
  { "CadetBlue",                   0x5f9ea0 },
  { "Chartreuse",                  0x7fff00 },
  { "Chocolate",                   0xd2691e },
  { "Coral",                       0xff7f50 },
  { "CornflowerBlue",              0x6495ed },
  { "Cornsilk",                    0xfff8dc },
  { "Crimson",                     0xdc143c },
  { "Cyan",                        0x00ffff },
  { "DarkBlue",                    0x00008b },
  { "DarkCyan",                    0x008b8b },
  { "DarkGoldenrod",               0xb8860b },
  { "DarkGray",                    0xa9a9a9 },
  { "DarkGreen",                   0x006400 },
  { "DarkGrey",                    0xa9a9a9 },
  { "DarkKhaki",                   0xbdb76b },
  { "DarkMagenta",                 0x8b008b },
  { "DarkOliveGreen",              0x556b2f },
  { "DarkOrange",                  0xff8c00 },
  { "DarkOrchid",                  0x9932cc },
  { "DarkRed",                     0x8b0000 },
  { "DarkSalmon",                  0xe9967a },
  { "DarkSeaGreen",                0x8fbc8f },
  { "DarkSlateBlue",               0x483d8b },
  { "DarkSlateGray",               0x2f4f4f },
  { "DarkSlateGrey",               0x2f4f4f },
  { "DarkTurquoise",               0x00ced1 },
  { "DarkViolet",                  0x9400d3 },
  { "DeepPink",                    0xff1493 },
  { "DeepSkyBlue",                 0x00bfff },
  { "DimGray",                     0x696969 },
  { "DimGrey",                     0x696969 },
  { "DodgerBlue",                  0x1e90ff },
  { "Firebrick",                   0xb22222 },
  { "FloralWhite",                 0xfffaf0 },
  { "ForestGreen",                 0x228b22 },
  { "Fuchsia",                     0xff00ff },
  { "Gainsboro",                   0xdcdcdc },
  { "GhostWhite",                  0xf8f8ff },
  { "Gold",                        0xffd700 },
  { "Goldenrod",                   0xdaa520 },
  { "Gray",                        0x808080 },
  { "Green",                       0x008000 },
  { "GreenYellow",                 0xadff2f },
  { "Grey",                        0x808080 },
  { "Honeydew",                    0xf0fff0 },
  { "HotPink",                     0xff69b4 },
  { "IndianRed",                   0xcd5c5c },
  { "Indigo",                      0x4b0082 },
  { "Ivory",                       0xfffff0 },
  { "Khaki",                       0xf0e68c },
  { "Lavender",                    0xe6e6fa },
  { "LavenderBlush",               0xfff0f5 },
  { "LawnGreen",                   0x7cfc00 },
  { "LemonChiffon",                0xfffacd },
  { "LightBlue",                   0xadd8e6 },
  { "LightCoral",                  0xf08080 },
  { "LightCyan",                   0xe0ffff },
  { "LightGoldenrodYellow",        0xfafad2 },
  { "LightGray",                   0xd3d3d3 },
  { "LightGreen",                  0x90ee90 },
  { "LightGrey",                   0xd3d3d3 },
  { "LightPink",                   0xffb6c1 },
  { "LightSalmon",                 0xffa07a },
  { "LightSeaGreen",               0x20b2aa },
  { "LightSkyBlue",                0x87cefa },
  { "LightSlateGray",              0x778899 },
  { "LightSlateGrey",              0x778899 },
  { "LightSteelBlue",              0xb0c4de },
  { "LightYellow",                 0xffffe0 },
  { "Lime",                        0x00ff00 },
  { "LimeGreen",                   0x32cd32 },
  { "Linen",                       0xfaf0e6 },
  { "Magenta",                     0xff00ff },
  { "Maroon",                      0x800000 },
  { "MediumAquamarine",            0x66cdaa },
  { "MediumBlue",                  0x0000cd },
  { "MediumOrchid",                0xba55d3 },
  { "MediumPurple",                0x9370db },
  { "MediumSeaGreen",              0x3cb371 },
  { "MediumSlateBlue",             0x7b68ee },
  { "MediumSpringGreen",           0x00fa9a },
  { "MediumTurquoise",             0x48d1cc },
  { "MediumVioletRed",             0xc71585 },
  { "MidnightBlue",                0x191970 },
  { "MintCream",                   0xf5fffa },
  { "MistyRose",                   0xffe4e1 },
  { "Moccasin",                    0xffe4b5 },
  { "NavajoWhite",                 0xffdead },
  { "Navy",                        0x000080 },
  { "None",                              -1 },  /* Non-standard addition */
  { "Off",                               -1 },  /* Non-standard addition */
  { "OldLace",                     0xfdf5e6 },
  { "Olive",                       0x808000 },
  { "OliveDrab",                   0x6b8e23 },
  { "Orange",                      0xffa500 },
  { "OrangeRed",                   0xff4500 },
  { "Orchid",                      0xda70d6 },
  { "PaleGoldenrod",               0xeee8aa },
  { "PaleGreen",                   0x98fb98 },
  { "PaleTurquoise",               0xafeeee },
  { "PaleVioletRed",               0xdb7093 },
  { "PapayaWhip",                  0xffefd5 },
  { "PeachPuff",                   0xffdab9 },
  { "Peru",                        0xcd853f },
  { "Pink",                        0xffc0cb },
  { "Plum",                        0xdda0dd },
  { "PowderBlue",                  0xb0e0e6 },
  { "Purple",                      0x800080 },
  { "RebeccaPurple",               0x663399 },
  { "Red",                         0xff0000 },
  { "RosyBrown",                   0xbc8f8f },
  { "RoyalBlue",                   0x4169e1 },
  { "SaddleBrown",                 0x8b4513 },
  { "Salmon",                      0xfa8072 },
  { "SandyBrown",                  0xf4a460 },
  { "SeaGreen",                    0x2e8b57 },
  { "Seashell",                    0xfff5ee },
  { "Sienna",                      0xa0522d },
  { "Silver",                      0xc0c0c0 },
  { "SkyBlue",                     0x87ceeb },
  { "SlateBlue",                   0x6a5acd },
  { "SlateGray",                   0x708090 },
  { "SlateGrey",                   0x708090 },
  { "Snow",                        0xfffafa },
  { "SpringGreen",                 0x00ff7f },
  { "SteelBlue",                   0x4682b4 },
  { "Tan",                         0xd2b48c },
  { "Teal",                        0x008080 },
  { "Thistle",                     0xd8bfd8 },
  { "Tomato",                      0xff6347 },
  { "Turquoise",                   0x40e0d0 },
  { "Violet",                      0xee82ee },
  { "Wheat",                       0xf5deb3 },
  { "White",                       0xffffff },
  { "WhiteSmoke",                  0xf5f5f5 },
  { "Yellow",                      0xffff00 },
  { "YellowGreen",                 0x9acd32 },
};

/* Built-in variable names.
**
** This array is constant.  When a script changes the value of one of
** these built-ins, a new PVar record is added at the head of
** the Pik.pVar list, which is searched first.  Thus the new PVar entry
** will override this default value.
**
** Units are in inches, except for "color" and "fill" which are 
** interpreted as 24-bit RGB values.
**
** Binary search used.  Must be kept in sorted order.
*/
static const struct { const char *zName; PNum val; } aBuiltin[] = {
  { "arcrad",      0.25  },
  { "arrowhead",   2.0   },
  { "arrowht",     0.08  },
  { "arrowwid",    0.06  },
  { "boxht",       0.5   },
  { "boxrad",      0.0   },
  { "boxwid",      0.75  },
  { "charht",      0.14  },
  { "charwid",     0.08  },
  { "circlerad",   0.25  },
  { "color",       0.0   },
  { "cylht",       0.5   },
  { "cylrad",      0.075 },
  { "cylwid",      0.75  },
  { "dashwid",     0.05  },
  { "diamondht",   0.75  },
  { "diamondwid",  1.0   },
  { "dotrad",      0.015 },
  { "ellipseht",   0.5   },
  { "ellipsewid",  0.75  },
  { "fileht",      0.75  },
  { "filerad",     0.15  },
  { "filewid",     0.5   },
  { "fill",        -1.0  },
  { "lineht",      0.5   },
  { "linewid",     0.5   },
  { "movewid",     0.5   },
  { "ovalht",      0.5   },
  { "ovalwid",     1.0   },
  { "scale",       1.0   },
  { "textht",      0.5   },
  { "textwid",     0.75  },
  { "thickness",   0.015 },
};


/* Methods for the "arc" class */
static void arcInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "arcrad",6,0);
  pObj->h = pObj->w;
}
/* Hack: Arcs are here rendered as quadratic Bezier curves rather
** than true arcs.  Multiple reasons: (1) the legacy-PIC parameters
** that control arcs are obscure and I could not figure out what they
** mean based on available documentation.  (2) Arcs are rarely used,
** and so do not seem that important.
*/
static PPoint arcControlPoint(int cw, PPoint f, PPoint t){
  PPoint m;
  PNum dx, dy;
  m.x = 0.5*(f.x+t.x);
  m.y = 0.5*(f.y+t.y);
  dx = t.x - f.x;
  dy = t.y - f.y;
  if( cw ){
    m.x -= 0.5*dy;
    m.y += 0.5*dx;
  }else{
    m.x += 0.5*dy;
    m.y -= 0.5*dx;
  }
  return m;
}
static void arcCheck(Pik *p, PObj *pObj){
  PPoint f, m, t;
  PNum sw;
  int i;
  if( p->nTPath>2 ){
    pik_error(p, &pObj->errTok, "arc geometry error");
    return;
  }
  f = p->aTPath[0];
  t = p->aTPath[1];
  m = arcControlPoint(pObj->cw, f, t);
  sw = pObj->sw;
  for(i=1; i<16; i++){
    PNum t1, t2, a, b, c, x, y;
    t1 = 0.0625*i;
    t2 = 1.0 - t1;
    a = t2*t2;
    b = 2*t1*t2;
    c = t1*t1;
    x = a*f.x + b*m.x + c*t.x;
    y = a*f.y + b*m.y + c*t.y;
    pik_bbox_addellipse(&pObj->bbox, x, y, sw, sw);
  }
}
static void arcRender(Pik *p, PObj *pObj){
  PPoint f, m, t;
  if( pObj->nPath<2 ) return;
  if( pObj->sw<0.0 ) return;
  f = pObj->aPath[0];
  t = pObj->aPath[1];
  m = arcControlPoint(pObj->cw,f,t);
  if( pObj->larrow ){
    pik_draw_arrowhead(p,&m,&f,pObj);
  }
  if( pObj->rarrow ){
    pik_draw_arrowhead(p,&m,&t,pObj);
  }
  pik_append_xy(p,"<path d=\"M", f.x, f.y);
  pik_append_xy(p,"Q", m.x, m.y);
  pik_append_xy(p," ", t.x, t.y);
  pik_append(p,"\" ",2);
  pik_append_style(p,pObj,0);
  pik_append(p,"\" />\n", -1);

  pik_append_txt(p, pObj, 0);
}


/* Methods for the "arrow" class */
static void arrowInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "linewid",7,0);
  pObj->h = pik_value(p, "lineht",6,0);
  pObj->rad = pik_value(p, "linerad",7,0);
  pObj->rarrow = 1;
}

/* Methods for the "box" class */
static void boxInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "boxwid",6,0);
  pObj->h = pik_value(p, "boxht",5,0);
  pObj->rad = pik_value(p, "boxrad",6,0);
}
/* Return offset from the center of the box to the compass point 
** given by parameter cp */
static PPoint boxOffset(Pik *p, PObj *pObj, int cp){
  PPoint pt = cZeroPoint;
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum rad = pObj->rad;
  PNum rx;
  if( rad<=0.0 ){
    rx = 0.0;
  }else{
    if( rad>w2 ) rad = w2;
    if( rad>h2 ) rad = h2;
    rx = 0.29289321881345252392*rad;
  }
  switch( cp ){
    case CP_C:                                   break;
    case CP_N:   pt.x = 0.0;      pt.y = h2;     break;
    case CP_NE:  pt.x = w2-rx;    pt.y = h2-rx;  break;
    case CP_E:   pt.x = w2;       pt.y = 0.0;    break;
    case CP_SE:  pt.x = w2-rx;    pt.y = rx-h2;  break;
    case CP_S:   pt.x = 0.0;      pt.y = -h2;    break;
    case CP_SW:  pt.x = rx-w2;    pt.y = rx-h2;  break;
    case CP_W:   pt.x = -w2;      pt.y = 0.0;    break;
    case CP_NW:  pt.x = rx-w2;    pt.y = h2-rx;  break;
    default:     assert(0);
  }
  UNUSED_PARAMETER(p);
  return pt;
}
static PPoint boxChop(Pik *p, PObj *pObj, PPoint *pPt){
  PNum dx, dy;
  int cp = CP_C;
  PPoint chop = pObj->ptAt;
  if( pObj->w<=0.0 ) return chop;
  if( pObj->h<=0.0 ) return chop;
  dx = (pPt->x - pObj->ptAt.x)*pObj->h/pObj->w;
  dy = (pPt->y - pObj->ptAt.y);
  if( dx>0.0 ){
    if( dy>=2.414*dx ){
      cp = CP_N;
    }else if( dy>=0.414*dx ){
      cp = CP_NE;
    }else if( dy>=-0.414*dx ){
      cp = CP_E;
    }else if( dy>-2.414*dx ){
      cp = CP_SE;
    }else{
      cp = CP_S;
    }
  }else{
    if( dy>=-2.414*dx ){
      cp = CP_N;
    }else if( dy>=-0.414*dx ){
      cp = CP_NW;
    }else if( dy>=0.414*dx ){
      cp = CP_W;
    }else if( dy>2.414*dx ){
      cp = CP_SW;
    }else{
      cp = CP_S;
    }
  }
  chop = pObj->type->xOffset(p,pObj,cp);
  chop.x += pObj->ptAt.x;
  chop.y += pObj->ptAt.y;
  return chop;
}
static void boxFit(Pik *p, PObj *pObj, PNum w, PNum h){
  if( w>0 ) pObj->w = w;
  if( h>0 ) pObj->h = h;
  UNUSED_PARAMETER(p);
}
static void boxRender(Pik *p, PObj *pObj){
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum rad = pObj->rad;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    if( rad<=0.0 ){
      pik_append_xy(p,"<path d=\"M", pt.x-w2,pt.y-h2);
      pik_append_xy(p,"L", pt.x+w2,pt.y-h2);
      pik_append_xy(p,"L", pt.x+w2,pt.y+h2);
      pik_append_xy(p,"L", pt.x-w2,pt.y+h2);
      pik_append(p,"Z\" ",-1);
    }else{
      /*
      **         ----       - y3
      **        /    \
      **       /      \     _ y2
      **      |        |    
      **      |        |    _ y1
      **       \      /
      **        \    /
      **         ----       _ y0
      **
      **      '  '  '  '
      **     x0 x1 x2 x3
      */
      PNum x0,x1,x2,x3,y0,y1,y2,y3;
      if( rad>w2 ) rad = w2;
      if( rad>h2 ) rad = h2;
      x0 = pt.x - w2;
      x1 = x0 + rad;
      x3 = pt.x + w2;
      x2 = x3 - rad;
      y0 = pt.y - h2;
      y1 = y0 + rad;
      y3 = pt.y + h2;
      y2 = y3 - rad;
      pik_append_xy(p,"<path d=\"M", x1, y0);
      if( x2>x1 ) pik_append_xy(p, "L", x2, y0);
      pik_append_arc(p, rad, rad, x3, y1);
      if( y2>y1 ) pik_append_xy(p, "L", x3, y2);
      pik_append_arc(p, rad, rad, x2, y3);
      if( x2>x1 ) pik_append_xy(p, "L", x1, y3);
      pik_append_arc(p, rad, rad, x0, y2);
      if( y2>y1 ) pik_append_xy(p, "L", x0, y1);
      pik_append_arc(p, rad, rad, x1, y0);
      pik_append(p,"Z\" ",-1);
    }
    pik_append_style(p,pObj,3);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}

/* Methods for the "circle" class */
static void circleInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "circlerad",9,0)*2;
  pObj->h = pObj->w;
  pObj->rad = 0.5*pObj->w;
}
static void circleNumProp(Pik *p, PObj *pObj, PToken *pId){
  /* For a circle, the width must equal the height and both must
  ** be twice the radius.  Enforce those constraints. */
  switch( pId->eType ){
    case T_DIAMETER:
    case T_RADIUS:
      pObj->w = pObj->h = 2.0*pObj->rad;
      break;
    case T_WIDTH:
      pObj->h = pObj->w;
      pObj->rad = 0.5*pObj->w;
      break;
    case T_HEIGHT:
      pObj->w = pObj->h;
      pObj->rad = 0.5*pObj->w;
      break;
  }
  UNUSED_PARAMETER(p);
}
static PPoint circleChop(Pik *p, PObj *pObj, PPoint *pPt){
  PPoint chop;
  PNum dx = pPt->x - pObj->ptAt.x;
  PNum dy = pPt->y - pObj->ptAt.y;
  PNum dist = hypot(dx,dy);
  if( dist<pObj->rad || dist<=0 ) return pObj->ptAt;
  chop.x = pObj->ptAt.x + dx*pObj->rad/dist;
  chop.y = pObj->ptAt.y + dy*pObj->rad/dist;
  UNUSED_PARAMETER(p);
  return chop;
}
static void circleFit(Pik *p, PObj *pObj, PNum w, PNum h){
  PNum mx = 0.0;
  if( w>0 ) mx = w;
  if( h>mx ) mx = h;
  if( w*h>0 && (w*w + h*h) > mx*mx ){
    mx = hypot(w,h);
  }
  if( mx>0.0 ){
    pObj->rad = 0.5*mx;
    pObj->w = pObj->h = mx;
  }
  UNUSED_PARAMETER(p);
}

static void circleRender(Pik *p, PObj *pObj){
  PNum r = pObj->rad;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    pik_append_x(p,"<circle cx=\"", pt.x, "\"");
    pik_append_y(p," cy=\"", pt.y, "\"");
    pik_append_dis(p," r=\"", r, "\" ");
    pik_append_style(p,pObj,3);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}

/* Methods for the "cylinder" class */
static void cylinderInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "cylwid",6,0);
  pObj->h = pik_value(p, "cylht",5,0);
  pObj->rad = pik_value(p, "cylrad",6,0); /* Minor radius of ellipses */
}
static void cylinderFit(Pik *p, PObj *pObj, PNum w, PNum h){
  if( w>0 ) pObj->w = w;
  if( h>0 ) pObj->h = h + 0.25*pObj->rad + pObj->sw;
  UNUSED_PARAMETER(p);
}
static void cylinderRender(Pik *p, PObj *pObj){
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum rad = pObj->rad;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    if( rad>h2 ){
      rad = h2;
    }else if( rad<0 ){
      rad = 0;
    }
    pik_append_xy(p,"<path d=\"M", pt.x-w2,pt.y+h2-rad);
    pik_append_xy(p,"L", pt.x-w2,pt.y-h2+rad);
    pik_append_arc(p,w2,rad,pt.x+w2,pt.y-h2+rad);
    pik_append_xy(p,"L", pt.x+w2,pt.y+h2-rad);
    pik_append_arc(p,w2,rad,pt.x-w2,pt.y+h2-rad);
    pik_append_arc(p,w2,rad,pt.x+w2,pt.y+h2-rad);
    pik_append(p,"\" ",-1);
    pik_append_style(p,pObj,3);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}
static PPoint cylinderOffset(Pik *p, PObj *pObj, int cp){
  PPoint pt = cZeroPoint;
  PNum w2 = pObj->w*0.5;
  PNum h1 = pObj->h*0.5;
  PNum h2 = h1 - pObj->rad;
  switch( cp ){
    case CP_C:                                break;
    case CP_N:   pt.x = 0.0;   pt.y = h1;     break;
    case CP_NE:  pt.x = w2;    pt.y = h2;     break;
    case CP_E:   pt.x = w2;    pt.y = 0.0;    break;
    case CP_SE:  pt.x = w2;    pt.y = -h2;    break;
    case CP_S:   pt.x = 0.0;   pt.y = -h1;    break;
    case CP_SW:  pt.x = -w2;   pt.y = -h2;    break;
    case CP_W:   pt.x = -w2;   pt.y = 0.0;    break;
    case CP_NW:  pt.x = -w2;   pt.y = h2;     break;
    default:     assert(0);
  }
  UNUSED_PARAMETER(p);
  return pt;
}

/* Methods for the "dot" class */
static void dotInit(Pik *p, PObj *pObj){
  pObj->rad = pik_value(p, "dotrad",6,0);
  pObj->h = pObj->w = pObj->rad*6;
  pObj->fill = pObj->color;
}
static void dotNumProp(Pik *p, PObj *pObj, PToken *pId){
  switch( pId->eType ){
    case T_COLOR:
      pObj->fill = pObj->color;
      break;
    case T_FILL:
      pObj->color = pObj->fill;
      break;
  }
  UNUSED_PARAMETER(p);
}
static void dotCheck(Pik *p, PObj *pObj){
  pObj->w = pObj->h = 0;
  pik_bbox_addellipse(&pObj->bbox, pObj->ptAt.x, pObj->ptAt.y,
                       pObj->rad, pObj->rad);
  UNUSED_PARAMETER(p);
}
static PPoint dotOffset(Pik *p, PObj *pObj, int cp){
  UNUSED_PARAMETER(p);
  UNUSED_PARAMETER(pObj);
  UNUSED_PARAMETER(cp);
  return cZeroPoint;
}
static void dotRender(Pik *p, PObj *pObj){
  PNum r = pObj->rad;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    pik_append_x(p,"<circle cx=\"", pt.x, "\"");
    pik_append_y(p," cy=\"", pt.y, "\"");
    pik_append_dis(p," r=\"", r, "\"");
    pik_append_style(p,pObj,2);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}

/* Methods for the "diamond" class */
static void diamondInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "diamondwid",10,0);
  pObj->h = pik_value(p, "diamondht",9,0);
  pObj->bAltAutoFit = 1;
}
/* Return offset from the center of the box to the compass point 
** given by parameter cp */
static PPoint diamondOffset(Pik *p, PObj *pObj, int cp){
  PPoint pt = cZeroPoint;
  PNum w2 = 0.5*pObj->w;
  PNum w4 = 0.25*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum h4 = 0.25*pObj->h;
  switch( cp ){
    case CP_C:                                   break;
    case CP_N:   pt.x = 0.0;      pt.y = h2;     break;
    case CP_NE:  pt.x = w4;       pt.y = h4;     break;
    case CP_E:   pt.x = w2;       pt.y = 0.0;    break;
    case CP_SE:  pt.x = w4;       pt.y = -h4;    break;
    case CP_S:   pt.x = 0.0;      pt.y = -h2;    break;
    case CP_SW:  pt.x = -w4;      pt.y = -h4;    break;
    case CP_W:   pt.x = -w2;      pt.y = 0.0;    break;
    case CP_NW:  pt.x = -w4;      pt.y = h4;     break;
    default:     assert(0);
  }
  UNUSED_PARAMETER(p);
  return pt;
}
static void diamondFit(Pik *p, PObj *pObj, PNum w, PNum h){
  if( pObj->w<=0 ) pObj->w = w*1.5;
  if( pObj->h<=0 ) pObj->h = h*1.5;
  if( pObj->w>0 && pObj->h>0 ){
    PNum x = pObj->w*h/pObj->h + w;
    PNum y = pObj->h*x/pObj->w;
    pObj->w = x;
    pObj->h = y;
  }
  UNUSED_PARAMETER(p);
}
static void diamondRender(Pik *p, PObj *pObj){
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    pik_append_xy(p,"<path d=\"M", pt.x-w2,pt.y);
    pik_append_xy(p,"L", pt.x,pt.y-h2);
    pik_append_xy(p,"L", pt.x+w2,pt.y);
    pik_append_xy(p,"L", pt.x,pt.y+h2);
    pik_append(p,"Z\" ",-1);
    pik_append_style(p,pObj,3);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}


/* Methods for the "ellipse" class */
static void ellipseInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "ellipsewid",10,0);
  pObj->h = pik_value(p, "ellipseht",9,0);
}
static PPoint ellipseChop(Pik *p, PObj *pObj, PPoint *pPt){
  PPoint chop;
  PNum s, dq, dist;
  PNum dx = pPt->x - pObj->ptAt.x;
  PNum dy = pPt->y - pObj->ptAt.y;
  if( pObj->w<=0.0 ) return pObj->ptAt;
  if( pObj->h<=0.0 ) return pObj->ptAt;
  s = pObj->h/pObj->w;
  dq = dx*s;
  dist = hypot(dq,dy);
  if( dist<pObj->h ) return pObj->ptAt;
  chop.x = pObj->ptAt.x + 0.5*dq*pObj->h/(dist*s);
  chop.y = pObj->ptAt.y + 0.5*dy*pObj->h/dist;
  UNUSED_PARAMETER(p);
  return chop;
}
static PPoint ellipseOffset(Pik *p, PObj *pObj, int cp){
  PPoint pt = cZeroPoint;
  PNum w = pObj->w*0.5;
  PNum w2 = w*0.70710678118654747608;
  PNum h = pObj->h*0.5;
  PNum h2 = h*0.70710678118654747608;
  switch( cp ){
    case CP_C:                                break;
    case CP_N:   pt.x = 0.0;   pt.y = h;      break;
    case CP_NE:  pt.x = w2;    pt.y = h2;     break;
    case CP_E:   pt.x = w;     pt.y = 0.0;    break;
    case CP_SE:  pt.x = w2;    pt.y = -h2;    break;
    case CP_S:   pt.x = 0.0;   pt.y = -h;     break;
    case CP_SW:  pt.x = -w2;   pt.y = -h2;    break;
    case CP_W:   pt.x = -w;    pt.y = 0.0;    break;
    case CP_NW:  pt.x = -w2;   pt.y = h2;     break;
    default:     assert(0);
  }
  UNUSED_PARAMETER(p);
  return pt;
}
static void ellipseRender(Pik *p, PObj *pObj){
  PNum w = pObj->w;
  PNum h = pObj->h;
  PPoint pt = pObj->ptAt;
  if( pObj->sw>=0.0 ){
    pik_append_x(p,"<ellipse cx=\"", pt.x, "\"");
    pik_append_y(p," cy=\"", pt.y, "\"");
    pik_append_dis(p," rx=\"", w/2.0, "\"");
    pik_append_dis(p," ry=\"", h/2.0, "\" ");
    pik_append_style(p,pObj,3);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}

/* Methods for the "file" object */
static void fileInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "filewid",7,0);
  pObj->h = pik_value(p, "fileht",6,0);
  pObj->rad = pik_value(p, "filerad",7,0);
}
/* Return offset from the center of the file to the compass point 
** given by parameter cp */
static PPoint fileOffset(Pik *p, PObj *pObj, int cp){
  PPoint pt = cZeroPoint;
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum rx = pObj->rad;
  PNum mn = w2<h2 ? w2 : h2;
  if( rx>mn ) rx = mn;
  if( rx<mn*0.25 ) rx = mn*0.25;
  pt.x = pt.y = 0.0;
  rx *= 0.5;
  switch( cp ){
    case CP_C:                                   break;
    case CP_N:   pt.x = 0.0;      pt.y = h2;     break;
    case CP_NE:  pt.x = w2-rx;    pt.y = h2-rx;  break;
    case CP_E:   pt.x = w2;       pt.y = 0.0;    break;
    case CP_SE:  pt.x = w2;       pt.y = -h2;    break;
    case CP_S:   pt.x = 0.0;      pt.y = -h2;    break;
    case CP_SW:  pt.x = -w2;      pt.y = -h2;    break;
    case CP_W:   pt.x = -w2;      pt.y = 0.0;    break;
    case CP_NW:  pt.x = -w2;      pt.y = h2;     break;
    default:     assert(0);
  }
  UNUSED_PARAMETER(p);
  return pt;
}
static void fileFit(Pik *p, PObj *pObj, PNum w, PNum h){
  if( w>0 ) pObj->w = w;
  if( h>0 ) pObj->h = h + 2*pObj->rad;
  UNUSED_PARAMETER(p);
}
static void fileRender(Pik *p, PObj *pObj){
  PNum w2 = 0.5*pObj->w;
  PNum h2 = 0.5*pObj->h;
  PNum rad = pObj->rad;
  PPoint pt = pObj->ptAt;
  PNum mn = w2<h2 ? w2 : h2;
  if( rad>mn ) rad = mn;
  if( rad<mn*0.25 ) rad = mn*0.25;
  if( pObj->sw>=0.0 ){
    pik_append_xy(p,"<path d=\"M", pt.x-w2,pt.y-h2);
    pik_append_xy(p,"L", pt.x+w2,pt.y-h2);
    pik_append_xy(p,"L", pt.x+w2,pt.y+(h2-rad));
    pik_append_xy(p,"L", pt.x+(w2-rad),pt.y+h2);
    pik_append_xy(p,"L", pt.x-w2,pt.y+h2);
    pik_append(p,"Z\" ",-1);
    pik_append_style(p,pObj,1);
    pik_append(p,"\" />\n",-1);
    pik_append_xy(p,"<path d=\"M", pt.x+(w2-rad), pt.y+h2);
    pik_append_xy(p,"L", pt.x+(w2-rad),pt.y+(h2-rad));
    pik_append_xy(p,"L", pt.x+w2, pt.y+(h2-rad));
    pik_append(p,"\" ",-1);
    pik_append_style(p,pObj,0);
    pik_append(p,"\" />\n",-1);
  }
  pik_append_txt(p, pObj, 0);
}


/* Methods for the "line" class */
static void lineInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "linewid",7,0);
  pObj->h = pik_value(p, "lineht",6,0);
  pObj->rad = pik_value(p, "linerad",7,0);
}
static PPoint lineOffset(Pik *p, PObj *pObj, int cp){
#if 0
  /* In legacy PIC, the .center of an unclosed line is half way between
  ** its .start and .end. */
  if( cp==CP_C && !pObj->bClose ){
    PPoint out;
    out.x = 0.5*(pObj->ptEnter.x + pObj->ptExit.x) - pObj->ptAt.x;
    out.y = 0.5*(pObj->ptEnter.x + pObj->ptExit.y) - pObj->ptAt.y;
    return out;
  }
#endif
  return boxOffset(p,pObj,cp);
}
static void lineRender(Pik *p, PObj *pObj){
  int i;
  if( pObj->sw>0.0 ){
    const char *z = "<path d=\"M";
    int n = pObj->nPath;
    if( pObj->larrow ){
      pik_draw_arrowhead(p,&pObj->aPath[1],&pObj->aPath[0],pObj);
    }
    if( pObj->rarrow ){
      pik_draw_arrowhead(p,&pObj->aPath[n-2],&pObj->aPath[n-1],pObj);
    }
    for(i=0; i<pObj->nPath; i++){
      pik_append_xy(p,z,pObj->aPath[i].x,pObj->aPath[i].y);
      z = "L";
    }
    if( pObj->bClose ){
      pik_append(p,"Z",1);
    }else{
      pObj->fill = -1.0;
    }
    pik_append(p,"\" ",-1);
    pik_append_style(p,pObj,pObj->bClose?3:0);
    pik_append(p,"\" />\n", -1);
  }
  pik_append_txt(p, pObj, 0);
}

/* Methods for the "move" class */
static void moveInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "movewid",7,0);
  pObj->h = pObj->w;
  pObj->fill = -1.0;
  pObj->color = -1.0;
  pObj->sw = -1.0;
}
static void moveRender(Pik *p, PObj *pObj){
  /* No-op */
  UNUSED_PARAMETER(p);
  UNUSED_PARAMETER(pObj);
}

/* Methods for the "oval" class */
static void ovalInit(Pik *p, PObj *pObj){
  pObj->h = pik_value(p, "ovalht",6,0);
  pObj->w = pik_value(p, "ovalwid",7,0);
  pObj->rad = 0.5*(pObj->h<pObj->w?pObj->h:pObj->w);
}
static void ovalNumProp(Pik *p, PObj *pObj, PToken *pId){
  UNUSED_PARAMETER(p);
  UNUSED_PARAMETER(pId);
  /* Always adjust the radius to be half of the smaller of
  ** the width and height. */
  pObj->rad = 0.5*(pObj->h<pObj->w?pObj->h:pObj->w);
}
static void ovalFit(Pik *p, PObj *pObj, PNum w, PNum h){
  UNUSED_PARAMETER(p);
  if( w>0 ) pObj->w = w;
  if( h>0 ) pObj->h = h;
  if( pObj->w<pObj->h ) pObj->w = pObj->h;
  pObj->rad = 0.5*(pObj->h<pObj->w?pObj->h:pObj->w);
}



/* Methods for the "spline" class */
static void splineInit(Pik *p, PObj *pObj){
  pObj->w = pik_value(p, "linewid",7,0);
  pObj->h = pik_value(p, "lineht",6,0);
  pObj->rad = 1000;
}
/* Return a point along the path from "f" to "t" that is r units
** prior to reaching "t", except if the path is less than 2*r total,
** return the midpoint.
*/
static PPoint radiusMidpoint(PPoint f, PPoint t, PNum r, int *pbMid){
  PNum dx = t.x - f.x;
  PNum dy = t.y - f.y;
  PNum dist = hypot(dx,dy);
  PPoint m;
  if( dist<=0.0 ) return t;
  dx /= dist;
  dy /= dist;
  if( r > 0.5*dist ){
    r = 0.5*dist;
    *pbMid = 1;
  }else{
    *pbMid = 0;
  }
  m.x = t.x - r*dx;
  m.y = t.y - r*dy;
  return m;
}
static void radiusPath(Pik *p, PObj *pObj, PNum r){
  int i;
  int n = pObj->nPath;
  const PPoint *a = pObj->aPath;
  PPoint m;
  PPoint an = a[n-1];
  int isMid = 0;
  int iLast = pObj->bClose ? n : n-1;

  pik_append_xy(p,"<path d=\"M", a[0].x, a[0].y);
  m = radiusMidpoint(a[0], a[1], r, &isMid);
  pik_append_xy(p," L ",m.x,m.y);
  for(i=1; i<iLast; i++){
    an = i<n-1 ? a[i+1] : a[0];
    m = radiusMidpoint(an,a[i],r, &isMid);
    pik_append_xy(p," Q ",a[i].x,a[i].y);
    pik_append_xy(p," ",m.x,m.y);
    if( !isMid ){
      m = radiusMidpoint(a[i],an,r, &isMid);
      pik_append_xy(p," L ",m.x,m.y);
    }
  }
  pik_append_xy(p," L ",an.x,an.y);
  if( pObj->bClose ){
    pik_append(p,"Z",1);
  }else{
    pObj->fill = -1.0;
  }
  pik_append(p,"\" ",-1);
  pik_append_style(p,pObj,pObj->bClose?3:0);
  pik_append(p,"\" />\n", -1);
}
static void splineRender(Pik *p, PObj *pObj){
  if( pObj->sw>0.0 ){
    int n = pObj->nPath;
    PNum r = pObj->rad;
    if( n<3 || r<=0.0 ){
      lineRender(p,pObj);
      return;
    }
    if( pObj->larrow ){
      pik_draw_arrowhead(p,&pObj->aPath[1],&pObj->aPath[0],pObj);
    }
    if( pObj->rarrow ){
      pik_draw_arrowhead(p,&pObj->aPath[n-2],&pObj->aPath[n-1],pObj);
    }
    radiusPath(p,pObj,pObj->rad);
  }
  pik_append_txt(p, pObj, 0);
}


/* Methods for the "text" class */
static void textInit(Pik *p, PObj *pObj){
  pik_value(p, "textwid",7,0);
  pik_value(p, "textht",6,0);
  pObj->sw = 0.0;
}
static PPoint textOffset(Pik *p, PObj *pObj, int cp){
  /* Automatically slim-down the width and height of text
  ** statements so that the bounding box tightly encloses the text,
  ** then get boxOffset() to do the offset computation.
  */
  pik_size_to_fit(p, pObj, &pObj->errTok,3);
  return boxOffset(p, pObj, cp);
}
static void textRender(Pik *p, PObj *pObj){
  pik_append_txt(p, pObj, 0);
}


/* Methods for the "sublist" class */
static void sublistInit(Pik *p, PObj *pObj){
  PList *pList = pObj->pSublist;
  int i;
  UNUSED_PARAMETER(p);
  pik_bbox_init(&pObj->bbox);
  for(i=0; i<pList->n; i++){
    pik_bbox_addbox(&pObj->bbox, &pList->a[i]->bbox);
  }
  pObj->w = pObj->bbox.ne.x - pObj->bbox.sw.x;
  pObj->h = pObj->bbox.ne.y - pObj->bbox.sw.y;
  pObj->ptAt.x = 0.5*(pObj->bbox.ne.x + pObj->bbox.sw.x);
  pObj->ptAt.y = 0.5*(pObj->bbox.ne.y + pObj->bbox.sw.y);
  pObj->mCalc |= A_WIDTH|A_HEIGHT|A_RADIUS;
}


/*
** The following array holds all the different kinds of objects.
** The special [] object is separate.
*/
static const PClass aClass[] = {
   {  /* name */          "arc",
      /* isline */        1,
      /* eJust */         0,
      /* xInit */         arcInit,
      /* xNumProp */      0,
      /* xCheck */        arcCheck,
      /* xChop */         0,
      /* xOffset */       boxOffset,
      /* xFit */          0,
      /* xRender */       arcRender
   },
   {  /* name */          "arrow",
      /* isline */        1,
      /* eJust */         0,
      /* xInit */         arrowInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       lineOffset,
      /* xFit */          0,
      /* xRender */       splineRender 
   },
   {  /* name */          "box",
      /* isline */        0,
      /* eJust */         1,
      /* xInit */         boxInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       boxOffset,
      /* xFit */          boxFit,
      /* xRender */       boxRender 
   },
   {  /* name */          "circle",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         circleInit,
      /* xNumProp */      circleNumProp,
      /* xCheck */        0,
      /* xChop */         circleChop,
      /* xOffset */       ellipseOffset,
      /* xFit */          circleFit,
      /* xRender */       circleRender 
   },
   {  /* name */          "cylinder",
      /* isline */        0,
      /* eJust */         1,
      /* xInit */         cylinderInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       cylinderOffset,
      /* xFit */          cylinderFit,
      /* xRender */       cylinderRender
   },
   {  /* name */          "diamond",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         diamondInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       diamondOffset,
      /* xFit */          diamondFit,
      /* xRender */       diamondRender 
   },
   {  /* name */          "dot",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         dotInit,
      /* xNumProp */      dotNumProp,
      /* xCheck */        dotCheck,
      /* xChop */         circleChop,
      /* xOffset */       dotOffset,
      /* xFit */          0,
      /* xRender */       dotRender 
   },
   {  /* name */          "ellipse",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         ellipseInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         ellipseChop,
      /* xOffset */       ellipseOffset,
      /* xFit */          boxFit,
      /* xRender */       ellipseRender
   },
   {  /* name */          "file",
      /* isline */        0,
      /* eJust */         1,
      /* xInit */         fileInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       fileOffset,
      /* xFit */          fileFit,
      /* xRender */       fileRender 
   },
   {  /* name */          "line",
      /* isline */        1,
      /* eJust */         0,
      /* xInit */         lineInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       lineOffset,
      /* xFit */          0,
      /* xRender */       splineRender
   },
   {  /* name */          "move",
      /* isline */        1,
      /* eJust */         0,
      /* xInit */         moveInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       boxOffset,
      /* xFit */          0,
      /* xRender */       moveRender
   },
   {  /* name */          "oval",
      /* isline */        0,
      /* eJust */         1,
      /* xInit */         ovalInit,
      /* xNumProp */      ovalNumProp,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       boxOffset,
      /* xFit */          ovalFit,
      /* xRender */       boxRender
   },
   {  /* name */          "spline",
      /* isline */        1,
      /* eJust */         0,
      /* xInit */         splineInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       lineOffset,
      /* xFit */          0,
      /* xRender */       splineRender
   },
   {  /* name */          "text",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         textInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         boxChop,
      /* xOffset */       textOffset,
      /* xFit */          boxFit,
      /* xRender */       textRender 
   },
};
static const PClass sublistClass = 
   {  /* name */          "[]",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         sublistInit,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       boxOffset,
      /* xFit */          0,
      /* xRender */       0 
   };
static const PClass noopClass = 
   {  /* name */          "noop",
      /* isline */        0,
      /* eJust */         0,
      /* xInit */         0,
      /* xNumProp */      0,
      /* xCheck */        0,
      /* xChop */         0,
      /* xOffset */       boxOffset,
      /* xFit */          0,
      /* xRender */       0
   };


/*
** Reduce the length of the line segment by amt (if possible) by
** modifying the location of *t.
*/
static void pik_chop(PPoint *f, PPoint *t, PNum amt){
  PNum dx = t->x - f->x;
  PNum dy = t->y - f->y;
  PNum dist = hypot(dx,dy);
  PNum r;
  if( dist<=amt ){
    *t = *f;
    return;
  }
  r = 1.0 - amt/dist;
  t->x = f->x + r*dx;
  t->y = f->y + r*dy;
}

/*
** Draw an arrowhead on the end of the line segment from pFrom to pTo.
** Also, shorten the line segment (by changing the value of pTo) so that
** the shaft of the arrow does not extend into the arrowhead.
*/
static void pik_draw_arrowhead(Pik *p, PPoint *f, PPoint *t, PObj *pObj){
  PNum dx = t->x - f->x;
  PNum dy = t->y - f->y;
  PNum dist = hypot(dx,dy);
  PNum h = p->hArrow * pObj->sw;
  PNum w = p->wArrow * pObj->sw;
  PNum e1, ddx, ddy;
  PNum bx, by;
  if( pObj->color<0.0 ) return;
  if( pObj->sw<=0.0 ) return;
  if( dist<=0.0 ) return;  /* Unable */
  dx /= dist;
  dy /= dist;
  e1 = dist - h;
  if( e1<0.0 ){
    e1 = 0.0;
    h = dist;
  }
  ddx = -w*dy;
  ddy = w*dx;
  bx = f->x + e1*dx;
  by = f->y + e1*dy;
  pik_append_xy(p,"<polygon points=\"", t->x, t->y);
  pik_append_xy(p," ",bx-ddx, by-ddy);
  pik_append_xy(p," ",bx+ddx, by+ddy);
  pik_append_clr(p,"\" style=\"fill:",pObj->color,"\"/>\n",0);
  pik_chop(f,t,h/2);
}

/*
** Compute the relative offset to an edge location from the reference for a
** an statement.
*/
static PPoint pik_elem_offset(Pik *p, PObj *pObj, int cp){
  return pObj->type->xOffset(p, pObj, cp);
}


/*
** Append raw text to zOut
*/
static void pik_append(Pik *p, const char *zText, int n){
  if( n<0 ) n = (int)strlen(zText);
  if( p->nOut+n>=p->nOutAlloc ){
    int nNew = (p->nOut+n)*2 + 1;
    char *z = realloc(p->zOut, nNew);
    if( z==0 ){
      pik_error(p, 0, 0);
      return;
    }
    p->zOut = z;
    p->nOutAlloc = nNew;
  }
  memcpy(p->zOut+p->nOut, zText, n);
  p->nOut += n;
  p->zOut[p->nOut] = 0;
}

/*
** Given a string and its length, returns true if the string begins
** with a construct which syntactically matches an HTML entity escape
** sequence (without checking for whether it's a known entity). Always
** returns false if zText[0] is false or n<4. Entities match the
** equivalent of the regexes `&#[0-9]{2,};` and
** `&[a-zA-Z][a-zA-Z0-9]+;`.
*/
static int pik_isentity(char const * zText, int n){
  int i = 0;
  if( n<4 || '&'!=zText[0] ) return 0;
  n--;
  zText++;
  if( '#'==zText[0] ){
    zText++;
    n--;
    for(i=0; i<n; i++){
      if( i>1 && ';'==zText[i] ) return 1;
      else if( zText[i]<'0' || zText[i]>'9' ) return 0;
      /* Note that &#nn; values nn<32d are not legal entities. */
    }
  }else{
    for(i=0; i<n; i++){
      if( i>1 && ';'==zText[i] ) return 1;
      else if( i>0 && zText[i]>='0' && zText[i]<='9' ){
          continue;
      }else if( zText[i]<'A' || zText[i]>'z'
               || (zText[i]>'Z' && zText[i]<'a') ) return 0;
    }
  }
  return 0;
}

/*
** Append text to zOut with HTML characters escaped.
**
**   *  The space character is changed into non-breaking space (U+00a0)
**      if mFlags has the 0x01 bit set. This is needed when outputting
**      text to preserve leading and trailing whitespace.  Turns out we
**      cannot use &nbsp; as that is an HTML-ism and is not valid in XML.
**
**   *  The "&" character is changed into "&amp;" if mFlags has the
**      0x02 bit set.  This is needed when generating error message text.
**
**   *  Except for the above, only "<" and ">" are escaped.
*/
static void pik_append_text(Pik *p, const char *zText, int n, int mFlags){
  int i;
  char c = 0;
  int bQSpace = mFlags & 1;
  int bQAmp = mFlags & 2;
  if( n<0 ) n = (int)strlen(zText);
  while( n>0 ){
    for(i=0; i<n; i++){
      c = zText[i];
      if( c=='<' || c=='>' ) break;
      if( c==' ' && bQSpace ) break;
      if( c=='&' && bQAmp ) break;
    }
    if( i ) pik_append(p, zText, i);
    if( i==n ) break;
    switch( c ){
      case '<': {  pik_append(p, "&lt;", 4);  break;  }
      case '>': {  pik_append(p, "&gt;", 4);  break;  }
      case ' ': {  pik_append(p, "\302\240;", 2);  break;  }
      case '&':
        if( pik_isentity(zText+i, n-i) ){ pik_append(p, "&", 1); }
        else { pik_append(p, "&amp;", 5); }
    }
    i++;
    n -= i;
    zText += i;
    i = 0;
  }
}

/*
** Append error message text.  This is either a raw append, or an append
** with HTML escapes, depending on whether the PIKCHR_PLAINTEXT_ERRORS flag
** is set.
*/
static void pik_append_errtxt(Pik *p, const char *zText, int n){
  if( p->mFlags & PIKCHR_PLAINTEXT_ERRORS ){
    pik_append(p, zText, n);
  }else{
    pik_append_text(p, zText, n, 0);
  }
}

/* Append a PNum value
*/
static void pik_append_num(Pik *p, const char *z,PNum v){
  char buf[100];
  snprintf(buf, sizeof(buf)-1, "%.10g", (double)v);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, z, -1);
  pik_append(p, buf, -1);
}

/* Append a PPoint value  (Used for debugging only)
*/
static void pik_append_point(Pik *p, const char *z, PPoint *pPt){
  char buf[100];
  snprintf(buf, sizeof(buf)-1, "%.10g,%.10g", 
          (double)pPt->x, (double)pPt->y);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, z, -1);
  pik_append(p, buf, -1);
}

/*
** Invert the RGB color so that it is appropriate for dark mode.
** Variable x hold the initial color.  The color is intended for use
** as a background color if isBg is true, and as a foreground color
** if isBg is false.
*/
static int pik_color_to_dark_mode(int x, int isBg){
  int r, g, b;
  int mn, mx;
  x = 0xffffff - x;
  r = (x>>16) & 0xff;
  g = (x>>8) & 0xff;
  b = x & 0xff;
  mx = r;
  if( g>mx ) mx = g;
  if( b>mx ) mx = b;
  mn = r;
  if( g<mn ) mn = g;
  if( b<mn ) mn = b;
  r = mn + (mx-r);
  g = mn + (mx-g);
  b = mn + (mx-b);
  if( isBg ){
    if( mx>127 ){
      r = (127*r)/mx;
      g = (127*g)/mx;
      b = (127*b)/mx;
    }
  }else{
    if( mn<128 && mx>mn ){
      r = 127 + ((r-mn)*128)/(mx-mn);
      g = 127 + ((g-mn)*128)/(mx-mn);
      b = 127 + ((b-mn)*128)/(mx-mn);
    }
  }
  return r*0x10000 + g*0x100 + b;
}

/* Append a PNum value surrounded by text.  Do coordinate transformations
** on the value.
*/
static void pik_append_x(Pik *p, const char *z1, PNum v, const char *z2){
  char buf[200];
  v -= p->bbox.sw.x;
  snprintf(buf, sizeof(buf)-1, "%s%g%s", z1, p->rScale*v, z2);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}
static void pik_append_y(Pik *p, const char *z1, PNum v, const char *z2){
  char buf[200];
  v = p->bbox.ne.y - v;
  snprintf(buf, sizeof(buf)-1, "%s%g%s", z1, p->rScale*v, z2);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}
static void pik_append_xy(Pik *p, const char *z1, PNum x, PNum y){
  char buf[200];
  x = x - p->bbox.sw.x;
  y = p->bbox.ne.y - y;
  snprintf(buf, sizeof(buf)-1, "%s%g,%g", z1, p->rScale*x, p->rScale*y);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}
static void pik_append_dis(Pik *p, const char *z1, PNum v, const char *z2){
  char buf[200];
  snprintf(buf, sizeof(buf)-1, "%s%g%s", z1, p->rScale*v, z2);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}

/* Append a color specification to the output.
**
** In PIKCHR_DARK_MODE, the color is inverted.  The "bg" flags indicates that
** the color is intended for use as a background color if true, or as a
** foreground color if false.  The distinction only matters for color
** inversions in PIKCHR_DARK_MODE.
*/
static void pik_append_clr(Pik *p,const char *z1,PNum v,const char *z2,int bg){
  char buf[200];
  int x = pik_round(v);
  int r, g, b;
  if( x==0 && p->fgcolor>0 && !bg ){
    x = p->fgcolor;
  }else if( bg && x>=0xffffff && p->bgcolor>0 ){
    x = p->bgcolor;
  }else if( p->mFlags & PIKCHR_DARK_MODE ){
    x = pik_color_to_dark_mode(x,bg);
  }
  r = (x>>16) & 0xff;
  g = (x>>8) & 0xff;
  b = x & 0xff;
  snprintf(buf, sizeof(buf)-1, "%srgb(%d,%d,%d)%s", z1, r, g, b, z2);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}

/* Append an SVG path A record:
**
**    A r1 r2 0 0 0 x y
*/
static void pik_append_arc(Pik *p, PNum r1, PNum r2, PNum x, PNum y){
  char buf[200];
  x = x - p->bbox.sw.x;
  y = p->bbox.ne.y - y;
  snprintf(buf, sizeof(buf)-1, "A%g %g 0 0 0 %g %g", 
     p->rScale*r1, p->rScale*r2,
     p->rScale*x, p->rScale*y);
  buf[sizeof(buf)-1] = 0;
  pik_append(p, buf, -1);
}

/* Append a style="..." text.  But, leave the quote unterminated, in case
** the caller wants to add some more.
**
** eFill is non-zero to fill in the background, or 0 if no fill should
** occur.  Non-zero values of eFill determine the "bg" flag to pik_append_clr()
** for cases when pObj->fill==pObj->color
**
**     1        fill is background, and color is foreground.
**     2        fill and color are both foreground.  (Used by "dot" objects)
**     3        fill and color are both background.  (Used by most other objs)
*/
static void pik_append_style(Pik *p, PObj *pObj, int eFill){
  int clrIsBg = 0;
  pik_append(p, " style=\"", -1);
  if( pObj->fill>=0 && eFill ){
    int fillIsBg = 1;
    if( pObj->fill==pObj->color ){
      if( eFill==2 ) fillIsBg = 0;
      if( eFill==3 ) clrIsBg = 1;
    }
    pik_append_clr(p, "fill:", pObj->fill, ";", fillIsBg);
  }else{
    pik_append(p,"fill:none;",-1);
  }
  if( pObj->sw>=0.0 && pObj->color>=0.0 ){
    PNum sw = pObj->sw;
    pik_append_dis(p, "stroke-width:", sw, ";");
    if( pObj->nPath>2 && pObj->rad<=pObj->sw ){
      pik_append(p, "stroke-linejoin:round;", -1);
    }
    pik_append_clr(p, "stroke:",pObj->color,";",clrIsBg);
    if( pObj->dotted>0.0 ){
      PNum v = pObj->dotted;
      if( sw<2.1/p->rScale ) sw = 2.1/p->rScale;
      pik_append_dis(p,"stroke-dasharray:",sw,"");
      pik_append_dis(p,",",v,";");
    }else if( pObj->dashed>0.0 ){
      PNum v = pObj->dashed;
      pik_append_dis(p,"stroke-dasharray:",v,"");
      pik_append_dis(p,",",v,";");
    }
  }
}

/*
** Compute the vertical locations for all text items in the
** object pObj.  In other words, set every pObj->aTxt[*].eCode
** value to contain exactly one of: TP_ABOVE2, TP_ABOVE, TP_CENTER,
** TP_BELOW, or TP_BELOW2 is set.
*/
static void pik_txt_vertical_layout(PObj *pObj){
  int n, i;
  PToken *aTxt;
  n = pObj->nTxt;
  if( n==0 ) return;
  aTxt = pObj->aTxt;
  if( n==1 ){
    if( (aTxt[0].eCode & TP_VMASK)==0 ){
      aTxt[0].eCode |= TP_CENTER;
    }
  }else{
    int allSlots = 0;
    int aFree[5];
    int iSlot;
    int j, mJust;
    /* If there is more than one TP_ABOVE, change the first to TP_ABOVE2. */
    for(j=mJust=0, i=n-1; i>=0; i--){
      if( aTxt[i].eCode & TP_ABOVE ){
        if( j==0 ){
          j++;
          mJust = aTxt[i].eCode & TP_JMASK;
        }else if( j==1 && mJust!=0 && (aTxt[i].eCode & mJust)==0 ){
          j++;
        }else{
          aTxt[i].eCode = (aTxt[i].eCode & ~TP_VMASK) | TP_ABOVE2;
          break;
        }
      }
    }
    /* If there is more than one TP_BELOW, change the last to TP_BELOW2 */
    for(j=mJust=0, i=0; i<n; i++){
      if( aTxt[i].eCode & TP_BELOW ){
        if( j==0 ){
          j++;
          mJust = aTxt[i].eCode & TP_JMASK;
        }else if( j==1 && mJust!=0 && (aTxt[i].eCode & mJust)==0 ){
          j++;
        }else{
          aTxt[i].eCode = (aTxt[i].eCode & ~TP_VMASK) | TP_BELOW2;
          break;
        }
      }
    }
    /* Compute a mask of all slots used */
    for(i=0; i<n; i++) allSlots |= aTxt[i].eCode & TP_VMASK;
    /* Set of an array of available slots */
    if( n==2
     && ((aTxt[0].eCode|aTxt[1].eCode)&TP_JMASK)==(TP_LJUST|TP_RJUST)
    ){
      /* Special case of two texts that have opposite justification:
      ** Allow them both to float to center. */
      iSlot = 2;
      aFree[0] = aFree[1] = TP_CENTER;
    }else{
      /* Set up the arrow so that available slots are filled from top to
      ** bottom */
      iSlot = 0;
      if( n>=4 && (allSlots & TP_ABOVE2)==0 ) aFree[iSlot++] = TP_ABOVE2;
      if( (allSlots & TP_ABOVE)==0 ) aFree[iSlot++] = TP_ABOVE;
      if( (n&1)!=0 ) aFree[iSlot++] = TP_CENTER;
      if( (allSlots & TP_BELOW)==0 ) aFree[iSlot++] = TP_BELOW;
      if( n>=4 && (allSlots & TP_BELOW2)==0 ) aFree[iSlot++] = TP_BELOW2;
    }
    /* Set the VMASK for all unassigned texts */
    for(i=iSlot=0; i<n; i++){
      if( (aTxt[i].eCode & TP_VMASK)==0 ){
        aTxt[i].eCode |= aFree[iSlot++];
      }
    }
  }
}

/* Return the font scaling factor associated with the input text attribute.
*/
static PNum pik_font_scale(PToken *t){
  PNum scale = 1.0;
  if( t->eCode & TP_BIG    ) scale *= 1.25;
  if( t->eCode & TP_SMALL  ) scale *= 0.8;
  if( t->eCode & TP_XTRA   ) scale *= scale;
  return scale;
}

/* Append multiple <text> SVG elements for the text fields of the PObj.
** Parameters:
**
**    p          The Pik object into which we are rendering
**
**    pObj       Object containing the text to be rendered
**
**    pBox       If not NULL, do no rendering at all.  Instead
**               expand the box object so that it will include all
**               of the text.
*/
static void pik_append_txt(Pik *p, PObj *pObj, PBox *pBox){
  PNum jw;          /* Justification margin relative to center */
  PNum ha2 = 0.0;   /* Height of the top row of text */
  PNum ha1 = 0.0;   /* Height of the second "above" row */
  PNum hc = 0.0;    /* Height of the center row */
  PNum hb1 = 0.0;   /* Height of the first "below" row of text */
  PNum hb2 = 0.0;   /* Height of the second "below" row */
  PNum yBase = 0.0;
  PNum sw = pObj->sw>=0.0 ? pObj->sw : 0;
  int n, i, nz;
  PNum x, y, orig_y, s;
  const char *z;
  PToken *aTxt;
  unsigned allMask = 0;

  if( p->nErr ) return;
  if( pObj->nTxt==0 ) return;
  aTxt = pObj->aTxt;
  n = pObj->nTxt;
  pik_txt_vertical_layout(pObj);
  x = pObj->ptAt.x;
  for(i=0; i<n; i++) allMask |= pObj->aTxt[i].eCode;
  if( pObj->type->isLine ){
    hc = sw*1.5;
  }else if( pObj->rad>0.0 && pObj->type->xInit==cylinderInit ){
    yBase = -0.75*pObj->rad;
  }
  if( allMask & TP_CENTER ){
    for(i=0; i<n; i++){
      if( pObj->aTxt[i].eCode & TP_CENTER ){
        s = pik_font_scale(pObj->aTxt+i);
        if( hc<s*p->charHeight ) hc = s*p->charHeight;
      }
    }
  }
  if( allMask & TP_ABOVE ){
    for(i=0; i<n; i++){
      if( pObj->aTxt[i].eCode & TP_ABOVE ){
        s = pik_font_scale(pObj->aTxt+i)*p->charHeight;
        if( ha1<s ) ha1 = s;
      }
    }
    if( allMask & TP_ABOVE2 ){
      for(i=0; i<n; i++){
        if( pObj->aTxt[i].eCode & TP_ABOVE2 ){
          s = pik_font_scale(pObj->aTxt+i)*p->charHeight;
          if( ha2<s ) ha2 = s;
        }
      }
    }
  }
  if( allMask & TP_BELOW ){
    for(i=0; i<n; i++){
      if( pObj->aTxt[i].eCode & TP_BELOW ){
        s = pik_font_scale(pObj->aTxt+i)*p->charHeight;
        if( hb1<s ) hb1 = s;
      }
    }
    if( allMask & TP_BELOW2 ){
      for(i=0; i<n; i++){
        if( pObj->aTxt[i].eCode & TP_BELOW2 ){
          s = pik_font_scale(pObj->aTxt+i)*p->charHeight;
          if( hb2<s ) hb2 = s;
        }
      }
    }
  }
  if( pObj->type->eJust==1 ){
    jw = 0.5*(pObj->w - 0.5*(p->charWidth + sw));
  }else{
    jw = 0.0;
  }
  for(i=0; i<n; i++){
    PToken *t = &aTxt[i];
    PNum xtraFontScale = pik_font_scale(t);
    PNum nx = 0;
    orig_y = pObj->ptAt.y;
    y = yBase;
    if( t->eCode & TP_ABOVE2 ) y += 0.5*hc + ha1 + 0.5*ha2;
    if( t->eCode & TP_ABOVE  ) y += 0.5*hc + 0.5*ha1;
    if( t->eCode & TP_BELOW  ) y -= 0.5*hc + 0.5*hb1;
    if( t->eCode & TP_BELOW2 ) y -= 0.5*hc + hb1 + 0.5*hb2;
    if( t->eCode & TP_LJUST  ) nx -= jw;
    if( t->eCode & TP_RJUST  ) nx += jw;

    if( pBox!=0 ){
      /* If pBox is not NULL, do not draw any <text>.  Instead, just expand
      ** pBox to include the text */
      PNum cw = pik_text_length(t, t->eCode & TP_MONO)*p->charWidth*xtraFontScale*0.01;
      PNum ch = p->charHeight*0.5*xtraFontScale;
      PNum x0, y0, x1, y1;  /* Boundary of text relative to pObj->ptAt */
      if( (t->eCode & (TP_BOLD|TP_MONO))==TP_BOLD ){
        cw *= 1.1;
      }
      if( t->eCode & TP_RJUST ){
        x0 = nx;
        y0 = y-ch;
        x1 = nx-cw;
        y1 = y+ch;
      }else if( t->eCode & TP_LJUST ){
        x0 = nx;
        y0 = y-ch;
        x1 = nx+cw;
        y1 = y+ch;
      }else{
        x0 = nx+cw/2;
        y0 = y+ch;
        x1 = nx-cw/2;
        y1 = y-ch;
      }
      if( (t->eCode & TP_ALIGN)!=0 && pObj->nPath>=2 ){
        int nn = pObj->nPath;
        PNum dx = pObj->aPath[nn-1].x - pObj->aPath[0].x;
        PNum dy = pObj->aPath[nn-1].y - pObj->aPath[0].y;
        if( dx!=0 || dy!=0 ){
          PNum dist = hypot(dx,dy);
          PNum tt;
          dx /= dist;
          dy /= dist;
          tt = dx*x0 - dy*y0;
          y0 = dy*x0 - dx*y0;
          x0 = tt;
          tt = dx*x1 - dy*y1;
          y1 = dy*x1 - dx*y1;
          x1 = tt;
        }
      }
      pik_bbox_add_xy(pBox, x+x0, orig_y+y0);
      pik_bbox_add_xy(pBox, x+x1, orig_y+y1);
      continue;
    }
    nx += x;
    y += orig_y;

    pik_append_x(p, "<text x=\"", nx, "\"");
    pik_append_y(p, " y=\"", y, "\"");
    if( t->eCode & TP_RJUST ){
      pik_append(p, " text-anchor=\"end\"", -1);
    }else if( t->eCode & TP_LJUST ){
      pik_append(p, " text-anchor=\"start\"", -1);
    }else{
      pik_append(p, " text-anchor=\"middle\"", -1);
    }
    if( t->eCode & TP_ITALIC ){
      pik_append(p, " font-style=\"italic\"", -1);
    }
    if( t->eCode & TP_BOLD ){
      pik_append(p, " font-weight=\"bold\"", -1);
    }
    if( t->eCode & TP_MONO ){
      pik_append(p, " font-family=\"monospace\"", -1);
    }
    if( pObj->color>=0.0 ){
      pik_append_clr(p, " fill=\"", pObj->color, "\"",0);
    }
    xtraFontScale *= p->fontScale;
    if( xtraFontScale<=0.99 || xtraFontScale>=1.01 ){
      pik_append_num(p, " font-size=\"", xtraFontScale*100.0);
      pik_append(p, "%\"", 2);
    }
    if( (t->eCode & TP_ALIGN)!=0 && pObj->nPath>=2 ){
      int nn = pObj->nPath;
      PNum dx = pObj->aPath[nn-1].x - pObj->aPath[0].x;
      PNum dy = pObj->aPath[nn-1].y - pObj->aPath[0].y;
      if( dx!=0 || dy!=0 ){
        PNum ang = atan2(dy,dx)*-180/M_PI;
        pik_append_num(p, " transform=\"rotate(", ang);
        pik_append_xy(p, " ", x, orig_y);
        pik_append(p,")\"",2);
      }
    }
    pik_append(p," dominant-baseline=\"central\">",-1);
    if( t->n>=2 && t->z[0]=='"' ){
      z = t->z+1;
      nz = t->n-2;
    }else{
      z = t->z;
      nz = t->n;
    }
    while( nz>0 ){
      int j;
      for(j=0; j<nz && z[j]!='\\'; j++){}
      if( j ) pik_append_text(p, z, j, 0x3);
      if( j<nz && (j+1==nz || z[j+1]=='\\') ){
        pik_append(p, "&#92;", -1);
        j++;
      }
      nz -= j+1;
      z += j+1;
    }
    pik_append(p, "</text>\n", -1);
  }
}

/*
** Append text (that will go inside of a <pre>...</pre>) that
** shows the context of an error token.
*/
static void pik_error_context(Pik *p, PToken *pErr, int nContext){
  int iErrPt;           /* Index of first byte of error from start of input */
  int iErrCol;          /* Column of the error token on its line */
  int iStart;           /* Start position of the error context */
  int iEnd;             /* End position of the error context */
  int iLineno;          /* Line number of the error */
  int iFirstLineno;     /* Line number of start of error context */
  int i;                /* Loop counter */
  int iBump = 0;        /* Bump the location of the error cursor */
  char zLineno[24];     /* Buffer in which to generate line numbers */

  iErrPt = (int)(pErr->z - p->sIn.z);
  if( iErrPt>=(int)p->sIn.n ){
    iErrPt = p->sIn.n-1;
    iBump = 1;
  }else{
    while( iErrPt>0 && (p->sIn.z[iErrPt]=='\n' || p->sIn.z[iErrPt]=='\r') ){
      iErrPt--;
      iBump = 1;
    }
  }
  iLineno = 1;
  for(i=0; i<iErrPt; i++){
    if( p->sIn.z[i]=='\n' ){
      iLineno++;
    }
  }
  iStart = 0;
  iFirstLineno = 1;
  while( iFirstLineno+nContext<iLineno ){
    while( p->sIn.z[iStart]!='\n' ){ iStart++; }
    iStart++;
    iFirstLineno++;
  }
  for(iEnd=iErrPt; p->sIn.z[iEnd]!=0 && p->sIn.z[iEnd]!='\n'; iEnd++){}
  i = iStart;
  while( iFirstLineno<=iLineno ){
    snprintf(zLineno,sizeof(zLineno)-1,"/* %4d */  ", iFirstLineno++);
    zLineno[sizeof(zLineno)-1] = 0;
    pik_append(p, zLineno, -1);
    for(i=iStart; p->sIn.z[i]!=0 && p->sIn.z[i]!='\n'; i++){}
    pik_append_errtxt(p, p->sIn.z+iStart, i-iStart);
    iStart = i+1;
    pik_append(p, "\n", 1);
  }
  for(iErrCol=0, i=iErrPt; i>0 && p->sIn.z[i]!='\n'; iErrCol++, i--){}
  for(i=0; i<iErrCol+11+iBump; i++){ pik_append(p, " ", 1); }
  for(i=0; i<(int)pErr->n; i++) pik_append(p, "^", 1);
  pik_append(p, "\n", 1);
}


/*
** Generate an error message for the output.  pErr is the token at which
** the error should point.  zMsg is the text of the error message. If
** either pErr or zMsg is NULL, generate an out-of-memory error message.
**
** This routine is a no-op if there has already been an error reported.
*/
static void pik_error(Pik *p, PToken *pErr, const char *zMsg){
  int i;
  if( p==0 ) return;
  if( p->nErr ) return;
  p->nErr++;
  if( zMsg==0 ){
    if( p->mFlags & PIKCHR_PLAINTEXT_ERRORS ){
      pik_append(p, "\nOut of memory\n", -1);
    }else{
      pik_append(p, "\n<div><p>Out of memory</p></div>\n", -1);
    }
    return;
  }
  if( pErr==0 ){
    pik_append(p, "\n", 1);
    pik_append_errtxt(p, zMsg, -1);
    return;
  }
  if( (p->mFlags & PIKCHR_PLAINTEXT_ERRORS)==0 ){
    pik_append(p, "<div><pre>\n", -1);
  }
  pik_error_context(p, pErr, 5);
  pik_append(p, "ERROR: ", -1);
  pik_append_errtxt(p, zMsg, -1);
  pik_append(p, "\n", 1);
  for(i=p->nCtx-1; i>=0; i--){
    pik_append(p, "Called from:\n", -1);
    pik_error_context(p, &p->aCtx[i], 0);
  }
  if( (p->mFlags & PIKCHR_PLAINTEXT_ERRORS)==0 ){
    pik_append(p, "</pre></div>\n", -1);
  }
}

/*
** Process an "assert( e1 == e2 )" statement.  Always return NULL.
*/
static PObj *pik_assert(Pik *p, PNum e1, PToken *pEq, PNum e2){
  char zE1[100], zE2[100], zMsg[300];

  /* Convert the numbers to strings using %g for comparison.  This
  ** limits the precision of the comparison to account for rounding error. */
  snprintf(zE1, sizeof(zE1), "%g", e1); zE1[sizeof(zE1)-1] = 0;
  snprintf(zE2, sizeof(zE2), "%g", e2); zE1[sizeof(zE2)-1] = 0;
  if( strcmp(zE1,zE2)!=0 ){
    snprintf(zMsg, sizeof(zMsg), "%.50s != %.50s", zE1, zE2);
    pik_error(p, pEq, zMsg);
  }
  return 0;
}

/*
** Process an "assert( place1 == place2 )" statement.  Always return NULL.
*/
static PObj *pik_position_assert(Pik *p, PPoint *e1, PToken *pEq, PPoint *e2){
  char zE1[100], zE2[100], zMsg[210];

  /* Convert the numbers to strings using %g for comparison.  This
  ** limits the precision of the comparison to account for rounding error. */
  snprintf(zE1, sizeof(zE1), "(%g,%g)", e1->x, e1->y); zE1[sizeof(zE1)-1] = 0;
  snprintf(zE2, sizeof(zE2), "(%g,%g)", e2->x, e2->y); zE1[sizeof(zE2)-1] = 0;
  if( strcmp(zE1,zE2)!=0 ){
    snprintf(zMsg, sizeof(zMsg), "%s != %s", zE1, zE2);
    pik_error(p, pEq, zMsg);
  }
  return 0;
}

/* Free a complete list of objects */
static void pik_elist_free(Pik *p, PList *pList){
  int i;
  if( pList==0 ) return;
  for(i=0; i<pList->n; i++){
    pik_elem_free(p, pList->a[i]);
  }
  free(pList->a);
  free(pList);
  return;
}

/* Free a single object, and its substructure */
static void pik_elem_free(Pik *p, PObj *pObj){
  if( pObj==0 ) return;
  free(pObj->zName);
  pik_elist_free(p, pObj->pSublist);
  free(pObj->aPath);
  free(pObj);
}

/* Convert a numeric literal into a number.  Return that number.
** There is no error handling because the tokenizer has already
** assured us that the numeric literal is valid.
**
** Allowed number forms:
**
**   (1)    Floating point literal
**   (2)    Same as (1) but followed by a unit: "cm", "mm", "in",
**          "px", "pt", or "pc".
**   (3)    Hex integers: 0x000000
**
** This routine returns the result in inches.  If a different unit
** is specified, the conversion happens automatically.
*/
PNum pik_atof(PToken *num){
  char *endptr;
  PNum ans;
  if( num->n>=3 && num->z[0]=='0' && (num->z[1]=='x'||num->z[1]=='X') ){
    return (PNum)strtol(num->z+2, 0, 16);
  }
  ans = strtod(num->z, &endptr);
  if( (int)(endptr - num->z)==(int)num->n-2 ){
    char c1 = endptr[0];
    char c2 = endptr[1];
    if( c1=='c' && c2=='m' ){
      ans /= 2.54;
    }else if( c1=='m' && c2=='m' ){
      ans /= 25.4;
    }else if( c1=='p' && c2=='x' ){
      ans /= 96;
    }else if( c1=='p' && c2=='t' ){
      ans /= 72;
    }else if( c1=='p' && c2=='c' ){
      ans /= 6;
    }
  }
  return ans;
}

/*
** Compute the distance between two points
*/
static PNum pik_dist(PPoint *pA, PPoint *pB){
  PNum dx, dy;
  dx = pB->x - pA->x;
  dy = pB->y - pA->y;
  return hypot(dx,dy);
}

/* Return true if a bounding box is empty.
*/
static int pik_bbox_isempty(PBox *p){
  return p->sw.x>p->ne.x;
}

/* Return true if point pPt is contained within the bounding box pBox
*/
static int pik_bbox_contains_point(PBox *pBox, PPoint *pPt){
  if( pik_bbox_isempty(pBox) ) return 0;
  if( pPt->x < pBox->sw.x ) return 0;
  if( pPt->x > pBox->ne.x ) return 0;
  if( pPt->y < pBox->sw.y ) return 0;
  if( pPt->y > pBox->ne.y ) return 0;
  return 1;
}

/* Initialize a bounding box to an empty container
*/
static void pik_bbox_init(PBox *p){
  p->sw.x = 1.0;
  p->sw.y = 1.0;
  p->ne.x = 0.0;
  p->ne.y = 0.0;
}

/* Enlarge the PBox of the first argument so that it fully
** covers the second PBox
*/
static void pik_bbox_addbox(PBox *pA, PBox *pB){
  if( pik_bbox_isempty(pA) ){
    *pA = *pB;
  }
  if( pik_bbox_isempty(pB) ) return;
  if( pA->sw.x>pB->sw.x ) pA->sw.x = pB->sw.x;
  if( pA->sw.y>pB->sw.y ) pA->sw.y = pB->sw.y;
  if( pA->ne.x<pB->ne.x ) pA->ne.x = pB->ne.x;
  if( pA->ne.y<pB->ne.y ) pA->ne.y = pB->ne.y;
}

/* Enlarge the PBox of the first argument, if necessary, so that
** it contains the point described by the 2nd and 3rd arguments.
*/
static void pik_bbox_add_xy(PBox *pA, PNum x, PNum y){
  if( pik_bbox_isempty(pA) ){
    pA->ne.x = x;
    pA->ne.y = y;
    pA->sw.x = x;
    pA->sw.y = y;
    return;
  }
  if( pA->sw.x>x ) pA->sw.x = x;
  if( pA->sw.y>y ) pA->sw.y = y;
  if( pA->ne.x<x ) pA->ne.x = x;
  if( pA->ne.y<y ) pA->ne.y = y;
}

/* Enlarge the PBox so that it is able to contain an ellipse
** centered at x,y and with radiuses rx and ry.
*/
static void pik_bbox_addellipse(PBox *pA, PNum x, PNum y, PNum rx, PNum ry){
  if( pik_bbox_isempty(pA) ){
    pA->ne.x = x+rx;
    pA->ne.y = y+ry;
    pA->sw.x = x-rx;
    pA->sw.y = y-ry;
    return;
  }
  if( pA->sw.x>x-rx ) pA->sw.x = x-rx;
  if( pA->sw.y>y-ry ) pA->sw.y = y-ry;
  if( pA->ne.x<x+rx ) pA->ne.x = x+rx;
  if( pA->ne.y<y+ry ) pA->ne.y = y+ry;
}



/* Append a new object onto the end of an object list.  The
** object list is created if it does not already exist.  Return
** the new object list.
*/
static PList *pik_elist_append(Pik *p, PList *pList, PObj *pObj){
  if( pObj==0 ) return pList;
  if( pList==0 ){
    pList = malloc(sizeof(*pList));
    if( pList==0 ){
      pik_error(p, 0, 0);
      pik_elem_free(p, pObj);
      return 0;
    }
    memset(pList, 0, sizeof(*pList));
  }
  if( pList->n>=pList->nAlloc ){
    int nNew = (pList->n+5)*2;
    PObj **pNew = realloc(pList->a, sizeof(PObj*)*nNew);
    if( pNew==0 ){
      pik_error(p, 0, 0);
      pik_elem_free(p, pObj);
      return pList;
    }
    pList->nAlloc = nNew;
    pList->a = pNew;
  }
  pList->a[pList->n++] = pObj;
  p->list = pList;
  return pList;
}

/* Convert an object class name into a PClass pointer
*/
static const PClass *pik_find_class(PToken *pId){
  int first = 0;
  int last = count(aClass) - 1;
  do{
    int mid = (first+last)/2;
    int c = strncmp(aClass[mid].zName, pId->z, pId->n);
    if( c==0 ){
      c = aClass[mid].zName[pId->n]!=0;
      if( c==0 ) return &aClass[mid];
    }
    if( c<0 ){
      first = mid + 1;
    }else{
      last = mid - 1;
    }
  }while( first<=last );
  return 0;
}

/* Allocate and return a new PObj object.
**
** If pId!=0 then pId is an identifier that defines the object class.
** If pStr!=0 then it is a STRING literal that defines a text object.
** If pSublist!=0 then this is a [...] object. If all three parameters
** are NULL then this is a no-op object used to define a PLACENAME.
*/
static PObj *pik_elem_new(Pik *p, PToken *pId, PToken *pStr,PList *pSublist){
  PObj *pNew;
  int miss = 0;

  if( p->nErr ) return 0;
  pNew = malloc( sizeof(*pNew) );
  if( pNew==0 ){
    pik_error(p,0,0);
    pik_elist_free(p, pSublist);
    return 0;
  }
  memset(pNew, 0, sizeof(*pNew));
  p->cur = pNew;
  p->nTPath = 1;
  p->thenFlag = 0;
  if( p->list==0 || p->list->n==0 ){
    pNew->ptAt.x = pNew->ptAt.y = 0.0;
    pNew->eWith = CP_C;
  }else{
    PObj *pPrior = p->list->a[p->list->n-1];
    pNew->ptAt = pPrior->ptExit;
    switch( p->eDir ){
      default:         pNew->eWith = CP_W;   break;
      case DIR_LEFT:   pNew->eWith = CP_E;   break;
      case DIR_UP:     pNew->eWith = CP_S;   break;
      case DIR_DOWN:   pNew->eWith = CP_N;   break;
    }
  }
  p->aTPath[0] = pNew->ptAt;
  pNew->with = pNew->ptAt;
  pNew->outDir = pNew->inDir = p->eDir;
  pNew->iLayer = pik_value_int(p, "layer", 5, &miss);
  if( miss ) pNew->iLayer = 1000;
  if( pNew->iLayer<0 ) pNew->iLayer = 0;
  if( pSublist ){
    pNew->type = &sublistClass;
    pNew->pSublist = pSublist;
    sublistClass.xInit(p,pNew);
    return pNew;
  }
  if( pStr ){
    PToken n;
    n.z = "text";
    n.n = 4;
    pNew->type = pik_find_class(&n);
    assert( pNew->type!=0 );
    pNew->errTok = *pStr;
    pNew->type->xInit(p, pNew);
    pik_add_txt(p, pStr, pStr->eCode);
    return pNew;
  }
  if( pId ){
    const PClass *pClass;
    pNew->errTok = *pId;
    pClass = pik_find_class(pId);
    if( pClass ){
      pNew->type = pClass;
      pNew->sw = pik_value(p, "thickness",9,0);
      pNew->fill = pik_value(p, "fill",4,0);
      pNew->color = pik_value(p, "color",5,0);
      pClass->xInit(p, pNew);
      return pNew;
    }
    pik_error(p, pId, "unknown object type");
    pik_elem_free(p, pNew);
    return 0;
  }
  pNew->type = &noopClass;
  pNew->ptExit = pNew->ptEnter = pNew->ptAt;
  return pNew;
}

/*
** If the ID token in the argument is the name of a macro, return
** the PMacro object for that macro
*/
static PMacro *pik_find_macro(Pik *p, PToken *pId){
  PMacro *pMac;
  for(pMac = p->pMacros; pMac; pMac=pMac->pNext){
    if( pMac->macroName.n==pId->n
     && strncmp(pMac->macroName.z,pId->z,pId->n)==0
    ){
      return pMac;
    }
  }
  return 0;
}

/* Add a new macro
*/
static void pik_add_macro(
  Pik *p,          /* Current Pikchr diagram */
  PToken *pId,     /* The ID token that defines the macro name */
  PToken *pCode    /* Macro body inside of {...} */
){
  PMacro *pNew = pik_find_macro(p, pId);
  if( pNew==0 ){
    pNew = malloc( sizeof(*pNew) );
    if( pNew==0 ){
      pik_error(p, 0, 0);
      return;
    }
    pNew->pNext = p->pMacros;
    p->pMacros = pNew;
    pNew->macroName = *pId;
  }
  pNew->macroBody.z = pCode->z+1;
  pNew->macroBody.n = pCode->n-2;
  pNew->inUse = 0;
}


/*
** Set the output direction and exit point for an object
*/
static void pik_elem_set_exit(PObj *pObj, int eDir){
  assert( ValidDir(eDir) );
  pObj->outDir = eDir;
  if( !pObj->type->isLine || pObj->bClose ){
    pObj->ptExit = pObj->ptAt;
    switch( pObj->outDir ){
      default:         pObj->ptExit.x += pObj->w*0.5;  break;
      case DIR_LEFT:   pObj->ptExit.x -= pObj->w*0.5;  break;
      case DIR_UP:     pObj->ptExit.y += pObj->h*0.5;  break;
      case DIR_DOWN:   pObj->ptExit.y -= pObj->h*0.5;  break;
    }
  }
}

/* Change the layout direction.
*/
static void pik_set_direction(Pik *p, int eDir){
  assert( ValidDir(eDir) );
  p->eDir = (unsigned char)eDir;

  /* It seems to make sense to reach back into the last object and
  ** change its exit point (its ".end") to correspond to the new
  ** direction.  Things just seem to work better this way.  However,
  ** legacy PIC does *not* do this.
  **
  ** The difference can be seen in a script like this:
  **
  **      arrow; circle; down; arrow
  **
  ** You can make pikchr render the above exactly like PIC
  ** by deleting the following three lines.  But I (drh) think
  ** it works better with those lines in place.
  */
  if( p->list && p->list->n ){
    pik_elem_set_exit(p->list->a[p->list->n-1], eDir);
  }
}

/* Move all coordinates contained within an object (and within its
** substructure) by dx, dy
*/
static void pik_elem_move(PObj *pObj, PNum dx, PNum dy){
  int i;
  pObj->ptAt.x += dx;
  pObj->ptAt.y += dy;
  pObj->ptEnter.x += dx;
  pObj->ptEnter.y += dy;
  pObj->ptExit.x += dx;
  pObj->ptExit.y += dy;
  pObj->bbox.ne.x += dx;
  pObj->bbox.ne.y += dy;
  pObj->bbox.sw.x += dx;
  pObj->bbox.sw.y += dy;
  for(i=0; i<pObj->nPath; i++){
    pObj->aPath[i].x += dx;
    pObj->aPath[i].y += dy;
  }
  if( pObj->pSublist ){
    pik_elist_move(pObj->pSublist, dx, dy);
  }
}
static void pik_elist_move(PList *pList, PNum dx, PNum dy){
  int i;
  for(i=0; i<pList->n; i++){
    pik_elem_move(pList->a[i], dx, dy);
  }
}

/*
** Check to see if it is ok to set the value of paraemeter mThis.
** Return 0 if it is ok. If it not ok, generate an appropriate
** error message and return non-zero.
**
** Flags are set in pObj so that the same object or conflicting
** objects may not be set again.
**
** To be ok, bit mThis must be clear and no more than one of
** the bits identified by mBlockers may be set.
*/
static int pik_param_ok(
  Pik *p,             /* For storing the error message (if any) */
  PObj *pObj,       /* The object under construction */
  PToken *pId,        /* Make the error point to this token */
  int mThis           /* Value we are trying to set */
){
  if( pObj->mProp & mThis ){
    pik_error(p, pId, "value is already set");
    return 1;
  }
  if( pObj->mCalc & mThis ){
    pik_error(p, pId, "value already fixed by prior constraints");
    return 1;
  }
  pObj->mProp |= mThis;
  return 0;
}


/*
** Set a numeric property like "width 7" or "radius 200%".
**
** The rAbs term is an absolute value to add in.  rRel is
** a relative value by which to change the current value.
*/
void pik_set_numprop(Pik *p, PToken *pId, PRel *pVal){
  PObj *pObj = p->cur;
  switch( pId->eType ){
    case T_HEIGHT:
      if( pik_param_ok(p, pObj, pId, A_HEIGHT) ) return;
      pObj->h = pObj->h*pVal->rRel + pVal->rAbs;
      break;
    case T_WIDTH:
      if( pik_param_ok(p, pObj, pId, A_WIDTH) ) return;
      pObj->w = pObj->w*pVal->rRel + pVal->rAbs;
      break;
    case T_RADIUS:
      if( pik_param_ok(p, pObj, pId, A_RADIUS) ) return;
      pObj->rad = pObj->rad*pVal->rRel + pVal->rAbs;
      break;
    case T_DIAMETER:
      if( pik_param_ok(p, pObj, pId, A_RADIUS) ) return;
      pObj->rad = pObj->rad*pVal->rRel + 0.5*pVal->rAbs; /* diam it 2x rad */
      break;
    case T_THICKNESS:
      if( pik_param_ok(p, pObj, pId, A_THICKNESS) ) return;
      pObj->sw = pObj->sw*pVal->rRel + pVal->rAbs;
      break;
  }
  if( pObj->type->xNumProp ){
    pObj->type->xNumProp(p, pObj, pId);
  }
  return;
}

/*
** Set a color property.  The argument is an RGB value.
*/
void pik_set_clrprop(Pik *p, PToken *pId, PNum rClr){
  PObj *pObj = p->cur;
  switch( pId->eType ){
    case T_FILL:
      if( pik_param_ok(p, pObj, pId, A_FILL) ) return;
      pObj->fill = rClr;
      break;
    case T_COLOR:
      if( pik_param_ok(p, pObj, pId, A_COLOR) ) return;
      pObj->color = rClr;
      break;
  }
  if( pObj->type->xNumProp ){
    pObj->type->xNumProp(p, pObj, pId);
  }
  return;
}

/*
** Set a "dashed" property like "dash 0.05"
**
** Use the value supplied by pVal if available.  If pVal==0, use
** a default.
*/
void pik_set_dashed(Pik *p, PToken *pId, PNum *pVal){
  PObj *pObj = p->cur;
  PNum v;
  switch( pId->eType ){
    case T_DOTTED:  {
      v = pVal==0 ? pik_value(p,"dashwid",7,0) : *pVal;
      pObj->dotted = v;
      pObj->dashed = 0.0;
      break;
    }
    case T_DASHED:  {
      v = pVal==0 ? pik_value(p,"dashwid",7,0) : *pVal;
      pObj->dashed = v;
      pObj->dotted = 0.0;
      break;
    }
  }
}

/*
** If the current path information came from a "same" or "same as"
** reset it.
*/
static void pik_reset_samepath(Pik *p){
  if( p->samePath ){
    p->samePath = 0;
    p->nTPath = 1;
  }
}


/* Add a new term to the path for a line-oriented object by transferring
** the information in the ptTo field over onto the path and into ptFrom
** resetting the ptTo.
*/
static void pik_then(Pik *p, PToken *pToken, PObj *pObj){
  int n;
  if( !pObj->type->isLine ){
    pik_error(p, pToken, "use with line-oriented objects only");
    return;
  }
  n = p->nTPath - 1;
  if( n<1 && (pObj->mProp & A_FROM)==0 ){
    pik_error(p, pToken, "no prior path points");
    return;
  }
  p->thenFlag = 1;
}

/* Advance to the next entry in p->aTPath.  Return its index.
*/
static int pik_next_rpath(Pik *p, PToken *pErr){
  int n = p->nTPath - 1;
  if( n+1>=(int)count(p->aTPath) ){
    pik_error(0, pErr, "too many path elements");
    return n;
  }
  n++;
  p->nTPath++;
  p->aTPath[n] = p->aTPath[n-1];
  p->mTPath = 0;
  return n;
}

/* Add a direction term to an object.  "up 0.5", or "left 3", or "down"
** or "down 50%".
*/
static void pik_add_direction(Pik *p, PToken *pDir, PRel *pVal){
  PObj *pObj = p->cur;
  int n;
  int dir;
  if( !pObj->type->isLine ){
    if( pDir ){
      pik_error(p, pDir, "use with line-oriented objects only");
    }else{
      PToken x = pik_next_semantic_token(&pObj->errTok);
      pik_error(p, &x, "syntax error");
    }
    return;
  }
  pik_reset_samepath(p);
  n = p->nTPath - 1;
  if( p->thenFlag || p->mTPath==3 || n==0 ){
    n = pik_next_rpath(p, pDir);
    p->thenFlag = 0;
  }
  dir = pDir ? pDir->eCode : p->eDir;
  switch( dir ){
    case DIR_UP:
       if( p->mTPath & 2 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].y += pVal->rAbs + pObj->h*pVal->rRel;
       p->mTPath |= 2;
       break;
    case DIR_DOWN:
       if( p->mTPath & 2 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].y -= pVal->rAbs + pObj->h*pVal->rRel;
       p->mTPath |= 2;
       break;
    case DIR_RIGHT:
       if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].x += pVal->rAbs + pObj->w*pVal->rRel;
       p->mTPath |= 1;
       break;
    case DIR_LEFT:
       if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].x -= pVal->rAbs + pObj->w*pVal->rRel;
       p->mTPath |= 1;
       break;
  }
  pObj->outDir = dir;
}

/* Process a movement attribute of one of these forms:
**
**         pDist   pHdgKW  rHdg    pEdgept
**     GO distance HEADING angle
**     GO distance               compasspoint
*/
static void pik_move_hdg(
  Pik *p,              /* The Pikchr context */
  PRel *pDist,         /* Distance to move */
  PToken *pHeading,    /* "heading" keyword if present */
  PNum rHdg,           /* Angle argument to "heading" keyword */
  PToken *pEdgept,     /* EDGEPT keyword "ne", "sw", etc... */
  PToken *pErr         /* Token to use for error messages */
){
  PObj *pObj = p->cur;
  int n;
  PNum rDist = pDist->rAbs + pik_value(p,"linewid",7,0)*pDist->rRel;
  if( !pObj->type->isLine ){
    pik_error(p, pErr, "use with line-oriented objects only");
    return;
  }
  pik_reset_samepath(p);
  do{
    n = pik_next_rpath(p, pErr);
  }while( n<1 );
  if( pHeading ){
    rHdg = fmod(rHdg,360.0);
  }else if( pEdgept->eEdge==CP_C ){
    pik_error(p, pEdgept, "syntax error");
    return;
  }else{
    rHdg = pik_hdg_angle[pEdgept->eEdge];
  }
  if( rHdg<=45.0 ){
    pObj->outDir = DIR_UP;
  }else if( rHdg<=135.0 ){
    pObj->outDir = DIR_RIGHT;
  }else if( rHdg<=225.0 ){
    pObj->outDir = DIR_DOWN;
  }else if( rHdg<=315.0 ){
    pObj->outDir = DIR_LEFT;
  }else{
    pObj->outDir = DIR_UP;
  }
  rHdg *= 0.017453292519943295769;  /* degrees to radians */
  p->aTPath[n].x += rDist*sin(rHdg);
  p->aTPath[n].y += rDist*cos(rHdg);
  p->mTPath = 2;
}


/* Process a movement attribute of the form "right until even with ..."
**
** pDir is the first keyword, "right" or "left" or "up" or "down".
** The movement is in that direction until its closest approach to
** the point specified by pPoint.
*/
static void pik_evenwith(Pik *p, PToken *pDir, PPoint *pPlace){
  PObj *pObj = p->cur;
  int n;
  if( !pObj->type->isLine ){
    pik_error(p, pDir, "use with line-oriented objects only");
    return;
  }
  pik_reset_samepath(p);
  n = p->nTPath - 1;
  if( p->thenFlag || p->mTPath==3 || n==0 ){
    n = pik_next_rpath(p, pDir);
    p->thenFlag = 0;
  }
  switch( pDir->eCode ){
    case DIR_DOWN:
    case DIR_UP:
       if( p->mTPath & 2 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].y = pPlace->y;
       p->mTPath |= 2;
       break;
    case DIR_RIGHT:
    case DIR_LEFT:
       if( p->mTPath & 1 ) n = pik_next_rpath(p, pDir);
       p->aTPath[n].x = pPlace->x;
       p->mTPath |= 1;
       break;
  }
  pObj->outDir = pDir->eCode;
}

/* If the last referenced object is centered at point pPt then return
** a pointer to that object.  If there is no prior object reference,
** or if the points are not the same, return NULL.
**
** This is a side-channel hack used to find the objects at which a
** line begins and ends.  For example, in
**
**        arrow from OBJ1 to OBJ2 chop
**
** The arrow object is normally just handed the coordinates of the
** centers for OBJ1 and OBJ2.  But we also want to know the specific
** object named in case there are multiple objects centered at the
** same point.
**
** See forum post 1d46e3a0bc
*/
static PObj *pik_last_ref_object(Pik *p, PPoint *pPt){
  PObj *pRes = 0;
  if( p->lastRef==0 ) return 0;
  if( p->lastRef->ptAt.x==pPt->x
   && p->lastRef->ptAt.y==pPt->y
  ){
    pRes = p->lastRef;
  }
  p->lastRef = 0;
  return pRes;
}

/* Set the "from" of an object
*/
static void pik_set_from(Pik *p, PObj *pObj, PToken *pTk, PPoint *pPt){
  if( !pObj->type->isLine ){
    pik_error(p, pTk, "use \"at\" to position this object");
    return;
  }
  if( pObj->mProp & A_FROM ){
    pik_error(p, pTk, "line start location already fixed");
    return;
  }
  if( pObj->bClose ){
    pik_error(p, pTk, "polygon is closed");
    return;
  }
  if( p->nTPath>1 ){
    PNum dx = pPt->x - p->aTPath[0].x;
    PNum dy = pPt->y - p->aTPath[0].y;
    int i;
    for(i=1; i<p->nTPath; i++){
      p->aTPath[i].x += dx;
      p->aTPath[i].y += dy;
    }
  }
  p->aTPath[0] = *pPt;
  p->mTPath = 3;
  pObj->mProp |= A_FROM;
  pObj->pFrom = pik_last_ref_object(p, pPt);
}

/* Set the "to" of an object
*/
static void pik_add_to(Pik *p, PObj *pObj, PToken *pTk, PPoint *pPt){
  int n = p->nTPath-1;
  if( !pObj->type->isLine ){
    pik_error(p, pTk, "use \"at\" to position this object");
    return;
  }
  if( pObj->bClose ){
    pik_error(p, pTk, "polygon is closed");
    return;
  }
  pik_reset_samepath(p);
  if( n==0 || p->mTPath==3 || p->thenFlag ){
    n = pik_next_rpath(p, pTk);
  }
  p->aTPath[n] = *pPt;
  p->mTPath = 3;
  pObj->pTo = pik_last_ref_object(p, pPt);
}

static void pik_close_path(Pik *p, PToken *pErr){
  PObj *pObj = p->cur;
  if( p->nTPath<3 ){
    pik_error(p, pErr,
      "need at least 3 vertexes in order to close the polygon");
    return;
  }
  if( pObj->bClose ){
    pik_error(p, pErr, "polygon already closed");
    return;
  }
  pObj->bClose = 1;
}

/* Lower the layer of the current object so that it is behind the
** given object.
*/
static void pik_behind(Pik *p, PObj *pOther){
  PObj *pObj = p->cur;
  if( p->nErr==0 && pObj->iLayer>=pOther->iLayer ){
    pObj->iLayer = pOther->iLayer - 1;
  }
}


/* Set the "at" of an object
*/
static void pik_set_at(Pik *p, PToken *pEdge, PPoint *pAt, PToken *pErrTok){
  PObj *pObj;
  static unsigned char eDirToCp[] = { CP_E, CP_S, CP_W, CP_N };
  if( p->nErr ) return;
  pObj = p->cur;

  if( pObj->type->isLine ){
    pik_error(p, pErrTok, "use \"from\" and \"to\" to position this object");
    return;
  }
  if( pObj->mProp & A_AT ){
    pik_error(p, pErrTok, "location fixed by prior \"at\"");
    return;
  }
  pObj->mProp |= A_AT;
  pObj->eWith = pEdge ? pEdge->eEdge : CP_C;
  if( pObj->eWith>=CP_END ){
    int dir = pObj->eWith==CP_END ? pObj->outDir : (pObj->inDir+2)%4;
    pObj->eWith = eDirToCp[dir];
  }
  pObj->with = *pAt;
}

/*
** Try to add a text attribute to an object
*/
static void pik_add_txt(Pik *p, PToken *pTxt, int iPos){
  PObj *pObj = p->cur;
  PToken *pT;
  if( pObj->nTxt >= count(pObj->aTxt) ){
    pik_error(p, pTxt, "too many text terms");
    return;
  }
  pT = &pObj->aTxt[pObj->nTxt++];
  *pT = *pTxt;
  pT->eCode = (short)iPos;
}

/* Merge "text-position" flags
*/
static int pik_text_position(int iPrev, PToken *pFlag){
  int iRes = iPrev;
  switch( pFlag->eType ){
    case T_LJUST:    iRes = (iRes&~TP_JMASK) | TP_LJUST;  break;
    case T_RJUST:    iRes = (iRes&~TP_JMASK) | TP_RJUST;  break;
    case T_ABOVE:    iRes = (iRes&~TP_VMASK) | TP_ABOVE;  break;
    case T_CENTER:   iRes = (iRes&~TP_VMASK) | TP_CENTER; break;
    case T_BELOW:    iRes = (iRes&~TP_VMASK) | TP_BELOW;  break;
    case T_ITALIC:   iRes |= TP_ITALIC;                   break;
    case T_BOLD:     iRes |= TP_BOLD;                     break;
    case T_MONO:     iRes |= TP_MONO;                     break;
    case T_ALIGNED:  iRes |= TP_ALIGN;                    break;
    case T_BIG:      if( iRes & TP_BIG ) iRes |= TP_XTRA;
                     else iRes = (iRes &~TP_SZMASK)|TP_BIG;   break;
    case T_SMALL:    if( iRes & TP_SMALL ) iRes |= TP_XTRA;
                     else iRes = (iRes &~TP_SZMASK)|TP_SMALL; break;
  }
  return iRes;
}

/*
** Table of scale-factor estimates for variable-width characters.
** Actual character widths vary by font.  These numbers are only
** guesses.  And this table only provides data for ASCII.
**
** 100 means normal width.
*/
static const unsigned char awChar[] = {
  /* Skip initial 32 control characters */
  /* ' ' */  45,
  /* '!' */  55,
  /* '"' */  62,
  /* '#' */  115,
  /* '$' */  90,
  /* '%' */  132,
  /* '&' */  125,
  /* '\''*/  40,

  /* '(' */  55,
  /* ')' */  55,
  /* '*' */  71,
  /* '+' */  115,
  /* ',' */  45,
  /* '-' */  48,
  /* '.' */  45,
  /* '/' */  50,

  /* '0' */  91,
  /* '1' */  91,
  /* '2' */  91,
  /* '3' */  91,
  /* '4' */  91,
  /* '5' */  91,
  /* '6' */  91,
  /* '7' */  91,

  /* '8' */  91,
  /* '9' */  91,
  /* ':' */  50,
  /* ';' */  50,
  /* '<' */ 120,
  /* '=' */ 120,
  /* '>' */ 120,
  /* '?' */  78,

  /* '@' */ 142,
  /* 'A' */ 102,
  /* 'B' */ 105,
  /* 'C' */ 110,
  /* 'D' */ 115,
  /* 'E' */ 105,
  /* 'F' */  98,
  /* 'G' */ 105,

  /* 'H' */ 125,
  /* 'I' */  58,
  /* 'J' */  58,
  /* 'K' */ 107,
  /* 'L' */  95,
  /* 'M' */ 145,
  /* 'N' */ 125,
  /* 'O' */ 115,

  /* 'P' */  95,
  /* 'Q' */ 115,
  /* 'R' */ 107,
  /* 'S' */  95,
  /* 'T' */  97,
  /* 'U' */ 118,
  /* 'V' */ 102,
  /* 'W' */ 150,

  /* 'X' */ 100,
  /* 'Y' */  93,
  /* 'Z' */ 100,
  /* '[' */  58,
  /* '\\'*/  50,
  /* ']' */  58,
  /* '^' */ 119,
  /* '_' */  72,

  /* '`' */  72,
  /* 'a' */  86,
  /* 'b' */  92,
  /* 'c' */  80,
  /* 'd' */  92,
  /* 'e' */  85,
  /* 'f' */  52,
  /* 'g' */  92,

  /* 'h' */  92,
  /* 'i' */  47,
  /* 'j' */  47,
  /* 'k' */  88,
  /* 'l' */  48,
  /* 'm' */ 135,
  /* 'n' */  92,
  /* 'o' */  86,

  /* 'p' */  92,
  /* 'q' */  92,
  /* 'r' */  69,
  /* 's' */  75,
  /* 't' */  58,
  /* 'u' */  92,
  /* 'v' */  80,
  /* 'w' */ 121,

  /* 'x' */  81,
  /* 'y' */  80,
  /* 'z' */  76,
  /* '{' */  91,
  /* '|'*/   49,
  /* '}' */  91,
  /* '~' */ 118,
};

/* Return an estimate of the width of the displayed characters
** in a character string.  The returned value is 100 times the
** average character width.
**
** Omit "\" used to escape characters.  And count entities like
** "&lt;" as a single character.  Multi-byte UTF8 characters count
** as a single character.
**
** Unless using a monospaced font, attempt to scale the answer by
** the actual characters seen.  Wide characters count more than
** narrow characters. But the widths are only guesses.
**
*/
static int pik_text_length(const PToken *pToken, const int isMonospace){
  const int stdAvg=100, monoAvg=82;
  int n = pToken->n;
  const char *z = pToken->z;
  int cnt, j;
  for(j=1, cnt=0; j<n-1; j++){
    char c = z[j];
    if( c=='\\' && z[j+1]!='&' ){
      c = z[++j];
    }else if( c=='&' ){
      int k;
      for(k=j+1; k<j+7 && z[k]!=0 && z[k]!=';'; k++){}
      if( z[k]==';' ) j = k;
      cnt += (isMonospace ? monoAvg : stdAvg) * 3 / 2;
      continue;
    }
    if( (c & 0xc0)==0xc0 ){
      while( j+1<n-1 && (z[j+1]&0xc0)==0x80 ){ j++; }
      cnt += isMonospace ? monoAvg : stdAvg;
      continue;
    }
    if( isMonospace ){
      cnt += monoAvg;
    }else if( c >= 0x20 && c <= 0x7e ){
      cnt += awChar[c-0x20];
    }else{
      cnt += stdAvg;
    }
  }
  return cnt;
}

/* Adjust the width, height, and/or radius of the object so that
** it fits around the text that has been added so far.
**
**    (1) Only text specified prior to this attribute is considered.
**    (2) The text size is estimated based on the charht and charwid
**        variable settings.
**    (3) The fitted attributes can be changed again after this
**        attribute, for example using "width 110%" if this auto-fit
**        underestimates the text size.
**    (4) Previously set attributes will not be altered.  In other words,
**        "width 1in fit" might cause the height to change, but the
**        width is now set.
**    (5) This only works for attributes that have an xFit method.
**
** The eWhich parameter is:
**
**    1:   Fit horizontally only
**    2:   Fit vertically only
**    3:   Fit both ways
*/
static void pik_size_to_fit(Pik *p, PObj *pObj, PToken *pFit, int eWhich){
  PNum w, h;
  PBox bbox;
  if( p->nErr ) return;
  if( pObj==0 ) pObj = p->cur;

  if( pObj->nTxt==0 ){
    pik_error(0, pFit, "no text to fit to");
    return;
  }
  if( pObj->type->xFit==0 ) return;
  pik_bbox_init(&bbox);
  pik_compute_layout_settings(p);
  pik_append_txt(p, pObj, &bbox);
  if( (eWhich & 1)!=0 || pObj->bAltAutoFit ){
    w = (bbox.ne.x - bbox.sw.x) + p->charWidth;
  }else{
    w = 0;
  }
  if( (eWhich & 2)!=0 || pObj->bAltAutoFit ){
    PNum h1, h2;
    h1 = (bbox.ne.y - pObj->ptAt.y);
    h2 = (pObj->ptAt.y - bbox.sw.y);
    h = 2.0*( h1<h2 ? h2 : h1 ) + 0.5*p->charHeight;
  }else{
    h = 0;
  }
  pObj->type->xFit(p, pObj, w, h);
  pObj->mProp |= A_FIT;
}

/* Set a local variable name to "val".
**
** The name might be a built-in variable or a color name.  In either case,
** a new application-defined variable is set.  Since app-defined variables
** are searched first, this will override any built-in variables.
*/
static void pik_set_var(Pik *p, PToken *pId, PNum val, PToken *pOp){
  PVar *pVar = p->pVar;
  while( pVar ){
    if( pik_token_eq(pId,pVar->zName)==0 ) break;
    pVar = pVar->pNext;
  }
  if( pVar==0 ){
    char *z;
    pVar = malloc( pId->n+1 + sizeof(*pVar) );
    if( pVar==0 ){
      pik_error(p, 0, 0);
      return;
    }
    pVar->zName = z = (char*)&pVar[1];
    memcpy(z, pId->z, pId->n);
    z[pId->n] = 0;
    pVar->pNext = p->pVar;
    pVar->val = pik_value(p, pId->z, pId->n, 0);
    p->pVar = pVar;
  }
  switch( pOp->eCode ){
    case T_PLUS:  pVar->val += val; break;
    case T_STAR:  pVar->val *= val; break;
    case T_MINUS: pVar->val -= val; break;
    case T_SLASH:
      if( val==0.0 ){
        pik_error(p, pOp, "division by zero");
      }else{
        pVar->val /= val;
      }
      break;
    default:      pVar->val = val; break;
  }
  p->bLayoutVars = 0;  /* Clear the layout setting cache */
}

/*
** Round a PNum into the nearest integer
*/
static int pik_round(PNum v){
  if( isnan(v) ) return 0;
  if( v < -2147483647 ) return (-2147483647-1);
  if( v >= 2147483647 ) return 2147483647;
  return (int)v;
}

/*
** Search for the variable named z[0..n-1] in:
**
**   * Application defined variables
**   * Built-in variables
**
** Return the value of the variable if found.  If not found
** return 0.0.  Also if pMiss is not NULL, then set it to 1
** if not found.
**
** This routine is a subroutine to pik_get_var().  But it is also
** used by object implementations to look up (possibly overwritten)
** values for built-in variables like "boxwid".
*/
static PNum pik_value(Pik *p, const char *z, int n, int *pMiss){
  PVar *pVar;
  int first, last, mid, c;
  for(pVar=p->pVar; pVar; pVar=pVar->pNext){
    if( strncmp(pVar->zName,z,n)==0 && pVar->zName[n]==0 ){
      return pVar->val;
    }
  }
  first = 0;
  last = count(aBuiltin)-1;
  while( first<=last ){
    mid = (first+last)/2;
    c = strncmp(z,aBuiltin[mid].zName,n);
    if( c==0 && aBuiltin[mid].zName[n] ) c = 1;
    if( c==0 ) return aBuiltin[mid].val;
    if( c>0 ){
      first = mid+1;
    }else{
      last = mid-1;
    }
  }
  if( pMiss ) *pMiss = 1;
  return 0.0;
}
static int pik_value_int(Pik *p, const char *z, int n, int *pMiss){
  return pik_round(pik_value(p,z,n,pMiss));
}

/*
** Look up a color-name.  Unlike other names in this program, the
** color-names are not case sensitive.  So "DarkBlue" and "darkblue"
** and "DARKBLUE" all find the same value (139).
**
** If not found, return -99.0.  Also post an error if p!=NULL.
**
** Special color names "None" and "Off" return -1.0 without causing
** an error.
*/
static PNum pik_lookup_color(Pik *p, PToken *pId){
  int first, last, mid, c = 0;
  first = 0;
  last = count(aColor)-1;
  while( first<=last ){
    const char *zClr;
    int c1, c2;
    unsigned int i;
    mid = (first+last)/2;
    zClr = aColor[mid].zName;
    for(i=0; i<pId->n; i++){
      c1 = zClr[i]&0x7f;
      if( IsUpper(c1) ) c1 = ToLower(c1);
      c2 = pId->z[i]&0x7f;
      if( IsUpper(c2) ) c2 = ToLower(c2);
      c = c2 - c1;
      if( c ) break;
    }
    if( c==0 && aColor[mid].zName[pId->n] ) c = -1;
    if( c==0 ) return (double)aColor[mid].val;
    if( c>0 ){
      first = mid+1;
    }else{
      last = mid-1;
    }
  }
  if( p ) pik_error(p, pId, "not a known color name");
  return -99.0;
}

/* Get the value of a variable.
**
** Search in order:
**
**    *  Application defined variables
**    *  Built-in variables
**    *  Color names
**
** If no such variable is found, throw an error.
*/
static PNum pik_get_var(Pik *p, PToken *pId){
  int miss = 0;
  PNum v = pik_value(p, pId->z, pId->n, &miss);
  if( miss==0 ) return v;
  v = pik_lookup_color(0, pId);
  if( v>-90.0 ) return v;
  pik_error(p,pId,"no such variable");
  return 0.0;
}

/* Convert a T_NTH token (ex: "2nd", "5th"} into a numeric value and
** return that value.  Throw an error if the value is too big.
*/
static short int pik_nth_value(Pik *p, PToken *pNth){
  int i = atoi(pNth->z);
  if( i>1000 ){
    pik_error(p, pNth, "value too big - max '1000th'");
    i = 1;
  }
  if( i==0 && pik_token_eq(pNth,"first")==0 ) i = 1;
  return (short int)i;
}

/* Search for the NTH object.
**
** If pBasis is not NULL then it should be a [] object.  Use the
** sublist of that [] object for the search.  If pBasis is not a []
** object, then throw an error.
**
** The pNth token describes the N-th search.  The pNth->eCode value
** is one more than the number of items to skip.  It is negative
** to search backwards.  If pNth->eType==T_ID, then it is the name
** of a class to search for.  If pNth->eType==T_LB, then
** search for a [] object.  If pNth->eType==T_LAST, then search for
** any type.
**
** Raise an error if the item is not found.
*/
static PObj *pik_find_nth(Pik *p, PObj *pBasis, PToken *pNth){
  PList *pList;
  int i, n;
  const PClass *pClass;
  if( pBasis==0 ){
    pList = p->list;
  }else{
    pList = pBasis->pSublist;
  }
  if( pList==0 ){
    pik_error(p, pNth, "no such object");
    return 0;
  }
  if( pNth->eType==T_LAST ){
    pClass = 0;
  }else if( pNth->eType==T_LB ){
    pClass = &sublistClass;
  }else{
    pClass = pik_find_class(pNth);
    if( pClass==0 ){
      pik_error(0, pNth, "no such object type");
      return 0;
    }
  }
  n = pNth->eCode;
  if( n<0 ){
    for(i=pList->n-1; i>=0; i--){
      PObj *pObj = pList->a[i];
      if( pClass && pObj->type!=pClass ) continue;
      n++;
      if( n==0 ){ return pObj; }
    }
  }else{
    for(i=0; i<pList->n; i++){
      PObj *pObj = pList->a[i];
      if( pClass && pObj->type!=pClass ) continue;
      n--;
      if( n==0 ){ return pObj; }
    }
  }
  pik_error(p, pNth, "no such object");
  return 0;
}

/* Search for an object by name.
**
** Search in pBasis->pSublist if pBasis is not NULL.  If pBasis is NULL
** then search in p->list.
*/
static PObj *pik_find_byname(Pik *p, PObj *pBasis, PToken *pName){
  PList *pList;
  int i, j;
  if( pBasis==0 ){
    pList = p->list;
  }else{
    pList = pBasis->pSublist;
  }
  if( pList==0 ){
    pik_error(p, pName, "no such object");
    return 0;
  }
  /* First look explicitly tagged objects */
  for(i=pList->n-1; i>=0; i--){
    PObj *pObj = pList->a[i];
    if( pObj->zName && pik_token_eq(pName,pObj->zName)==0 ){
      p->lastRef = pObj;
      return pObj;
    }
  }
  /* If not found, do a second pass looking for any object containing
  ** text which exactly matches pName */
  for(i=pList->n-1; i>=0; i--){
    PObj *pObj = pList->a[i];
    for(j=0; j<pObj->nTxt; j++){
      if( pObj->aTxt[j].n==pName->n+2
       && memcmp(pObj->aTxt[j].z+1,pName->z,pName->n)==0 ){
        p->lastRef = pObj;
        return pObj;
      }
    }
  }
  pik_error(p, pName, "no such object");
  return 0;
}

/* Change most of the settings for the current object to be the
** same as the pOther object, or the most recent object of the same
** type if pOther is NULL.
*/
static void pik_same(Pik *p, PObj *pOther, PToken *pErrTok){
  PObj *pObj = p->cur;
  if( p->nErr ) return;
  if( pOther==0 ){
    int i;
    for(i=(p->list ? p->list->n : 0)-1; i>=0; i--){
      pOther = p->list->a[i];
      if( pOther->type==pObj->type ) break;
    }
    if( i<0 ){
      pik_error(p, pErrTok, "no prior objects of the same type");
      return;
    }
  }
  if( pOther->nPath && pObj->type->isLine ){
    PNum dx, dy;
    int i;
    dx = p->aTPath[0].x - pOther->aPath[0].x;
    dy = p->aTPath[0].y - pOther->aPath[0].y;
    for(i=1; i<pOther->nPath; i++){
      p->aTPath[i].x = pOther->aPath[i].x + dx;
      p->aTPath[i].y = pOther->aPath[i].y + dy;
    }
    p->nTPath = pOther->nPath;
    p->mTPath = 3;
    p->samePath = 1;
  }
  if( !pObj->type->isLine ){
    pObj->w = pOther->w;
    pObj->h = pOther->h;
  }
  pObj->rad = pOther->rad;
  pObj->sw = pOther->sw;
  pObj->dashed = pOther->dashed;
  pObj->dotted = pOther->dotted;
  pObj->fill = pOther->fill;
  pObj->color = pOther->color;
  pObj->cw = pOther->cw;
  pObj->larrow = pOther->larrow;
  pObj->rarrow = pOther->rarrow;
  pObj->bClose = pOther->bClose;
  pObj->bChop = pOther->bChop;
  pObj->iLayer = pOther->iLayer;
}


/* Return a "Place" associated with object pObj.  If pEdge is NULL
** return the center of the object.  Otherwise, return the corner
** described by pEdge.
*/
static PPoint pik_place_of_elem(Pik *p, PObj *pObj, PToken *pEdge){
  PPoint pt = cZeroPoint;
  const PClass *pClass;
  if( pObj==0 ) return pt;
  if( pEdge==0 ){
    return pObj->ptAt;
  }
  pClass = pObj->type;
  if( pEdge->eType==T_EDGEPT || (pEdge->eEdge>0 && pEdge->eEdge<CP_END) ){
    pt = pClass->xOffset(p, pObj, pEdge->eEdge);
    pt.x += pObj->ptAt.x;
    pt.y += pObj->ptAt.y;
    return pt;
  }
  if( pEdge->eType==T_START ){
    return pObj->ptEnter;
  }else{
    return pObj->ptExit;
  }
}

/* Do a linear interpolation of two positions.
*/
static PPoint pik_position_between(PNum x, PPoint p1, PPoint p2){
  PPoint out;
  out.x = p2.x*x + p1.x*(1.0 - x);
  out.y = p2.y*x + p1.y*(1.0 - x);
  return out;
}

/* Compute the position that is dist away from pt at an heading angle of r
**
** The angle is a compass heading in degrees.  North is 0 (or 360).
** East is 90.  South is 180.  West is 270.  And so forth.
*/
static PPoint pik_position_at_angle(PNum dist, PNum r, PPoint pt){
  r *= 0.017453292519943295769;  /* degrees to radians */
  pt.x += dist*sin(r);
  pt.y += dist*cos(r);
  return pt;
}

/* Compute the position that is dist away at a compass point
*/
static PPoint pik_position_at_hdg(PNum dist, PToken *pD, PPoint pt){
  return pik_position_at_angle(dist, pik_hdg_angle[pD->eEdge], pt);
}

/* Return the coordinates for the n-th vertex of a line.
*/
static PPoint pik_nth_vertex(Pik *p, PToken *pNth, PToken *pErr, PObj *pObj){
  static const PPoint zero = {0, 0};
  int n;
  if( p->nErr || pObj==0 ) return p->aTPath[0];
  if( !pObj->type->isLine ){
    pik_error(p, pErr, "object is not a line");
    return zero;
  }
  n = atoi(pNth->z);
  if( n<1 || n>pObj->nPath ){
    pik_error(p, pNth, "no such vertex");
    return zero;
  }
  return pObj->aPath[n-1];
}

/* Return the value of a property of an object.
*/
static PNum pik_property_of(PObj *pObj, PToken *pProp){
  PNum v = 0.0;
  if( pObj ){
    switch( pProp->eType ){
      case T_HEIGHT:    v = pObj->h;            break;
      case T_WIDTH:     v = pObj->w;            break;
      case T_RADIUS:    v = pObj->rad;          break;
      case T_DIAMETER:  v = pObj->rad*2.0;      break;
      case T_THICKNESS: v = pObj->sw;           break;
      case T_DASHED:    v = pObj->dashed;       break;
      case T_DOTTED:    v = pObj->dotted;       break;
      case T_FILL:      v = pObj->fill;         break;
      case T_COLOR:     v = pObj->color;        break;
      case T_X:         v = pObj->ptAt.x;       break;
      case T_Y:         v = pObj->ptAt.y;       break;
      case T_TOP:       v = pObj->bbox.ne.y;    break;
      case T_BOTTOM:    v = pObj->bbox.sw.y;    break;
      case T_LEFT:      v = pObj->bbox.sw.x;    break;
      case T_RIGHT:     v = pObj->bbox.ne.x;    break;
    }
  }
  return v;
}

/* Compute one of the built-in functions
*/
static PNum pik_func(Pik *p, PToken *pFunc, PNum x, PNum y){
  PNum v = 0.0;
  switch( pFunc->eCode ){
    case FN_ABS:  v = x<0.0 ? -x : x;  break;
    case FN_COS:  v = cos(x);          break;
    case FN_INT:  v = rint(x);         break;
    case FN_SIN:  v = sin(x);          break;
    case FN_SQRT:
      if( x<0.0 ){
        pik_error(p, pFunc, "sqrt of negative value");
        v = 0.0;
      }else{
        v = sqrt(x);
      }
      break;
    case FN_MAX:  v = x>y ? x : y;   break;
    case FN_MIN:  v = x<y ? x : y;   break;
    default:      v = 0.0;
  }
  return v;
}

/* Attach a name to an object
*/
static void pik_elem_setname(Pik *p, PObj *pObj, PToken *pName){
  if( pObj==0 ) return;
  if( pName==0 ) return;
  free(pObj->zName);
  pObj->zName = malloc(pName->n+1);
  if( pObj->zName==0 ){
    pik_error(p,0,0);
  }else{
    memcpy(pObj->zName,pName->z,pName->n);
    pObj->zName[pName->n] = 0;
  }
  return;
}

/*
** Search for object located at *pCenter that has an xChop method and
** that does not enclose point pOther.
**
** Return a pointer to the object, or NULL if not found.
*/
static PObj *pik_find_chopper(PList *pList, PPoint *pCenter, PPoint *pOther){
  int i;
  if( pList==0 ) return 0;
  for(i=pList->n-1; i>=0; i--){
    PObj *pObj = pList->a[i];
    if( pObj->type->xChop!=0
     && pObj->ptAt.x==pCenter->x
     && pObj->ptAt.y==pCenter->y
     && !pik_bbox_contains_point(&pObj->bbox, pOther)
    ){
      return pObj;
    }else if( pObj->pSublist ){
      pObj = pik_find_chopper(pObj->pSublist,pCenter,pOther);
      if( pObj ) return pObj;
    }
  }
  return 0;
}

/*
** There is a line traveling from pFrom to pTo.
**
** If pObj is not null and is a choppable object, then chop at
** the boundary of pObj - where the line crosses the boundary
** of pObj.
**
** If pObj is NULL or has no xChop method, then search for some
** other object centered at pTo that is choppable and use it
** instead.
*/
static void pik_autochop(Pik *p, PPoint *pFrom, PPoint *pTo, PObj *pObj){
  if( pObj==0 || pObj->type->xChop==0 ){
    pObj = pik_find_chopper(p->list, pTo, pFrom);
  }
  if( pObj ){
    *pTo = pObj->type->xChop(p, pObj, pFrom);
  }
}

/* This routine runs after all attributes have been received
** on an object.
*/
static void pik_after_adding_attributes(Pik *p, PObj *pObj){
  int i;
  PPoint ofst;
  PNum dx, dy;

  if( p->nErr ) return;

  /* Position block objects */
  if( pObj->type->isLine==0 ){
    /* A height or width less than or equal to zero means "autofit".
    ** Change the height or width to be big enough to contain the text,
    */
    if( pObj->h<=0.0 ){
      if( pObj->nTxt==0 ){
        pObj->h = 0.0;
      }else if( pObj->w<=0.0 ){
        pik_size_to_fit(p, pObj, &pObj->errTok, 3);
      }else{
        pik_size_to_fit(p, pObj, &pObj->errTok, 2);
      }
    }
    if( pObj->w<=0.0 ){
      if( pObj->nTxt==0 ){
        pObj->w = 0.0;
      }else{
        pik_size_to_fit(p, pObj, &pObj->errTok, 1);
      }
    }
    ofst = pik_elem_offset(p, pObj, pObj->eWith);
    dx = (pObj->with.x - ofst.x) - pObj->ptAt.x;
    dy = (pObj->with.y - ofst.y) - pObj->ptAt.y;
    if( dx!=0 || dy!=0 ){
      pik_elem_move(pObj, dx, dy);
    }
  }

  /* For a line object with no movement specified, a single movement
  ** of the default length in the current direction
  */
  if( pObj->type->isLine && p->nTPath<2 ){
    pik_next_rpath(p, 0);
    assert( p->nTPath==2 );
    switch( pObj->inDir ){
      default:        p->aTPath[1].x += pObj->w; break;
      case DIR_DOWN:  p->aTPath[1].y -= pObj->h; break;
      case DIR_LEFT:  p->aTPath[1].x -= pObj->w; break;
      case DIR_UP:    p->aTPath[1].y += pObj->h; break;
    }
    if( pObj->type->xInit==arcInit ){
      pObj->outDir = (pObj->inDir + (pObj->cw ? 1 : 3))%4;
      p->eDir = (unsigned char)pObj->outDir;
      switch( pObj->outDir ){
        default:        p->aTPath[1].x += pObj->w; break;
        case DIR_DOWN:  p->aTPath[1].y -= pObj->h; break;
        case DIR_LEFT:  p->aTPath[1].x -= pObj->w; break;
        case DIR_UP:    p->aTPath[1].y += pObj->h; break;
      }
    }
  }

  /* Initialize the bounding box prior to running xCheck */
  pik_bbox_init(&pObj->bbox);

  /* Run object-specific code */
  if( pObj->type->xCheck!=0 ){
    pObj->type->xCheck(p,pObj);
    if( p->nErr ) return;
  }

  /* Compute final bounding box, entry and exit points, center
  ** point (ptAt) and path for the object
  */
  if( pObj->type->isLine ){
    pObj->aPath = malloc( sizeof(PPoint)*p->nTPath );
    if( pObj->aPath==0 ){
      pik_error(p, 0, 0);
      return;
    }else{
      pObj->nPath = p->nTPath;
      for(i=0; i<p->nTPath; i++){
        pObj->aPath[i] = p->aTPath[i];
      }
    }

    /* "chop" processing:
    ** If the line goes to the center of an object with an
    ** xChop method, then use the xChop method to trim the line.
    */
    if( pObj->bChop && pObj->nPath>=2 ){
      int n = pObj->nPath;
      pik_autochop(p, &pObj->aPath[n-2], &pObj->aPath[n-1], pObj->pTo);
      pik_autochop(p, &pObj->aPath[1], &pObj->aPath[0], pObj->pFrom);
    }

    pObj->ptEnter = pObj->aPath[0];
    pObj->ptExit = pObj->aPath[pObj->nPath-1];

    /* Compute the center of the line based on the bounding box over
    ** the vertexes.  This is a difference from PIC.  In Pikchr, the
    ** center of a line is the center of its bounding box. In PIC, the
    ** center of a line is halfway between its .start and .end.  For
    ** straight lines, this is the same point, but for multi-segment
    ** lines the result is usually diferent */
    for(i=0; i<pObj->nPath; i++){
      pik_bbox_add_xy(&pObj->bbox, pObj->aPath[i].x, pObj->aPath[i].y);
    }
    pObj->ptAt.x = (pObj->bbox.ne.x + pObj->bbox.sw.x)/2.0;
    pObj->ptAt.y = (pObj->bbox.ne.y + pObj->bbox.sw.y)/2.0;

    /* Reset the width and height of the object to be the width and height
    ** of the bounding box over vertexes */
    pObj->w = pObj->bbox.ne.x - pObj->bbox.sw.x;
    pObj->h = pObj->bbox.ne.y - pObj->bbox.sw.y;

    /* If this is a polygon (if it has the "close" attribute), then
    ** adjust the exit point */
    if( pObj->bClose ){
      /* For "closed" lines, the .end is one of the .e, .s, .w, or .n
      ** points of the bounding box, as with block objects. */
      pik_elem_set_exit(pObj, pObj->inDir);
    }
  }else{
    PNum w2 = pObj->w/2.0;
    PNum h2 = pObj->h/2.0;
    pObj->ptEnter = pObj->ptAt;
    pObj->ptExit = pObj->ptAt;
    switch( pObj->inDir ){
      default:         pObj->ptEnter.x -= w2;  break;
      case DIR_LEFT:   pObj->ptEnter.x += w2;  break;
      case DIR_UP:     pObj->ptEnter.y -= h2;  break;
      case DIR_DOWN:   pObj->ptEnter.y += h2;  break;
    }
    switch( pObj->outDir ){
      default:         pObj->ptExit.x += w2;  break;
      case DIR_LEFT:   pObj->ptExit.x -= w2;  break;
      case DIR_UP:     pObj->ptExit.y += h2;  break;
      case DIR_DOWN:   pObj->ptExit.y -= h2;  break;
    }
    pik_bbox_add_xy(&pObj->bbox, pObj->ptAt.x - w2, pObj->ptAt.y - h2);
    pik_bbox_add_xy(&pObj->bbox, pObj->ptAt.x + w2, pObj->ptAt.y + h2);
  }
  p->eDir = (unsigned char)pObj->outDir;
}

/* Show basic information about each object as a comment in the
** generated HTML.  Used for testing and debugging.  Activated
** by the (undocumented) "debug = 1;"
** command.
*/
static void pik_elem_render(Pik *p, PObj *pObj){
  char *zDir;
  if( pObj==0 ) return;
  pik_append(p,"<!-- ", -1);
  if( pObj->zName ){
    pik_append_text(p, pObj->zName, -1, 0);
    pik_append(p, ": ", 2);
  }
  pik_append_text(p, pObj->type->zName, -1, 0);
  if( pObj->nTxt ){
    pik_append(p, " \"", 2);
    pik_append_text(p, pObj->aTxt[0].z+1, pObj->aTxt[0].n-2, 1);
    pik_append(p, "\"", 1);
  }
  pik_append_num(p, " w=", pObj->w);
  pik_append_num(p, " h=", pObj->h);
  pik_append_point(p, " center=", &pObj->ptAt);
  pik_append_point(p, " enter=", &pObj->ptEnter);
  switch( pObj->outDir ){
    default:        zDir = " right";  break;
    case DIR_LEFT:  zDir = " left";   break;
    case DIR_UP:    zDir = " up";     break;
    case DIR_DOWN:  zDir = " down";   break;
  }
  pik_append_point(p, " exit=", &pObj->ptExit);
  pik_append(p, zDir, -1);
  pik_append(p, " -->\n", -1);
}

/* Render a list of objects
*/
void pik_elist_render(Pik *p, PList *pList){
  int i;
  int iNextLayer = 0;
  int iThisLayer;
  int bMoreToDo;
  int miss = 0;
  int mDebug = pik_value_int(p, "debug", 5, 0);
  PNum colorLabel;
  do{
    bMoreToDo = 0;
    iThisLayer = iNextLayer;
    iNextLayer = 0x7fffffff;
    for(i=0; i<pList->n; i++){
      PObj *pObj = pList->a[i];
      void (*xRender)(Pik*,PObj*);
      if( pObj->iLayer>iThisLayer ){
        if( pObj->iLayer<iNextLayer ) iNextLayer = pObj->iLayer;
        bMoreToDo = 1;
        continue; /* Defer until another round */
      }else if( pObj->iLayer<iThisLayer ){
        continue;
      }
      if( mDebug & 1 ) pik_elem_render(p, pObj);
      xRender = pObj->type->xRender;
      if( xRender ){
        xRender(p, pObj);
      }
      if( pObj->pSublist ){
        pik_elist_render(p, pObj->pSublist);
      }
    }
  }while( bMoreToDo );

  /* If the color_debug_label value is defined, then go through
  ** and paint a dot at every label location */
  colorLabel = pik_value(p, "debug_label_color", 17, &miss);
  if( miss==0 && colorLabel>=0.0 ){
    PObj dot;
    memset(&dot, 0, sizeof(dot));
    dot.type = &noopClass;
    dot.rad = 0.015;
    dot.sw = 0.015;
    dot.fill = colorLabel;
    dot.color = colorLabel;
    dot.nTxt = 1;
    dot.aTxt[0].eCode = TP_ABOVE;
    for(i=0; i<pList->n; i++){
      PObj *pObj = pList->a[i];
      if( pObj->zName==0 ) continue;
      dot.ptAt = pObj->ptAt;
      dot.aTxt[0].z = pObj->zName;
      dot.aTxt[0].n = (int)strlen(pObj->zName);
      dotRender(p, &dot);
    }
  }
}

/* Add all objects of the list pList to the bounding box
*/
static void pik_bbox_add_elist(Pik *p, PList *pList, PNum wArrow){
  int i;
  for(i=0; i<pList->n; i++){
    PObj *pObj = pList->a[i];
    if( pObj->sw>=0.0 ) pik_bbox_addbox(&p->bbox, &pObj->bbox);
    pik_append_txt(p, pObj, &p->bbox);
    if( pObj->pSublist ) pik_bbox_add_elist(p, pObj->pSublist, wArrow);


    /* Expand the bounding box to account for arrowheads on lines */
    if( pObj->type->isLine && pObj->nPath>0 ){
      if( pObj->larrow ){
        pik_bbox_addellipse(&p->bbox, pObj->aPath[0].x, pObj->aPath[0].y,
                            wArrow, wArrow);
      }
      if( pObj->rarrow ){
        int j = pObj->nPath-1;
        pik_bbox_addellipse(&p->bbox, pObj->aPath[j].x, pObj->aPath[j].y,
                            wArrow, wArrow);
      }
    }
  }
}

/* Recompute key layout parameters from variables. */
static void pik_compute_layout_settings(Pik *p){
  PNum thickness;  /* Line thickness */
  PNum wArrow;     /* Width of arrowheads */

  /* Set up rendering parameters */
  if( p->bLayoutVars ) return;
  thickness = pik_value(p,"thickness",9,0);
  if( thickness<=0.01 ) thickness = 0.01;
  wArrow = 0.5*pik_value(p,"arrowwid",8,0);
  p->wArrow = wArrow/thickness;
  p->hArrow = pik_value(p,"arrowht",7,0)/thickness;
  p->fontScale = pik_value(p,"fontscale",9,0);
  if( p->fontScale<=0.0 ) p->fontScale = 1.0;
  p->rScale = 144.0;
  p->charWidth = pik_value(p,"charwid",7,0)*p->fontScale;
  p->charHeight = pik_value(p,"charht",6,0)*p->fontScale;
  p->bLayoutVars = 1;
}

/* Render a list of objects.  Write the SVG into p->zOut.
** Delete the input object_list before returnning.
*/
static void pik_render(Pik *p, PList *pList){
  if( pList==0 ) return;
  if( p->nErr==0 ){
    PNum thickness;  /* Stroke width */
    PNum margin;     /* Extra bounding box margin */
    PNum w, h;       /* Drawing width and height */
    PNum wArrow;
    PNum pikScale;   /* Value of the "scale" variable */
    int miss = 0;

    /* Set up rendering parameters */
    pik_compute_layout_settings(p);
    thickness = pik_value(p,"thickness",9,0);
    if( thickness<=0.01 ) thickness = 0.01;
    margin = pik_value(p,"margin",6,0);
    margin += thickness;
    wArrow = p->wArrow*thickness;
    miss = 0;
    p->fgcolor = pik_value_int(p,"fgcolor",7,&miss);
    if( miss ){
      PToken t;
      t.z = "fgcolor";
      t.n = 7;
      p->fgcolor = pik_round(pik_lookup_color(0, &t));
    }
    miss = 0;
    p->bgcolor = pik_value_int(p,"bgcolor",7,&miss);
    if( miss ){
      PToken t;
      t.z = "bgcolor";
      t.n = 7;
      p->bgcolor = pik_round(pik_lookup_color(0, &t));
    }

    /* Compute a bounding box over all objects so that we can know
    ** how big to declare the SVG canvas */
    pik_bbox_init(&p->bbox);
    pik_bbox_add_elist(p, pList, wArrow);

    /* Expand the bounding box slightly to account for line thickness
    ** and the optional "margin = EXPR" setting. */
    p->bbox.ne.x += margin + pik_value(p,"rightmargin",11,0);
    p->bbox.ne.y += margin + pik_value(p,"topmargin",9,0);
    p->bbox.sw.x -= margin + pik_value(p,"leftmargin",10,0);
    p->bbox.sw.y -= margin + pik_value(p,"bottommargin",12,0);

    /* Output the SVG */
    pik_append(p, "<svg xmlns='http://www.w3.org/2000/svg'"
                  " style='font-size:initial;'",-1);
    if( p->zClass ){
      pik_append(p, " class=\"", -1);
      pik_append(p, p->zClass, -1);
      pik_append(p, "\"", 1);
    }
    w = p->bbox.ne.x - p->bbox.sw.x;
    h = p->bbox.ne.y - p->bbox.sw.y;
    p->wSVG = pik_round(p->rScale*w);
    p->hSVG = pik_round(p->rScale*h);
    pikScale = pik_value(p,"scale",5,0);
    if( pikScale>=0.001 && pikScale<=1000.0
     && (pikScale<0.99 || pikScale>1.01)
    ){
      p->wSVG = pik_round(p->wSVG*pikScale);
      p->hSVG = pik_round(p->hSVG*pikScale);
      pik_append_num(p, " width=\"", p->wSVG);
      pik_append_num(p, "\" height=\"", p->hSVG);
      pik_append(p, "\"", 1);
    }
    pik_append_dis(p, " viewBox=\"0 0 ",w,"");
    pik_append_dis(p, " ",h,"\"");
    pik_append(p, " data-pikchr-date=\"" MANIFEST_ISODATE "\">\n", -1);
    pik_elist_render(p, pList);
    pik_append(p,"</svg>\n", -1);
  }else{
    p->wSVG = -1;
    p->hSVG = -1;
  }
  pik_elist_free(p, pList);
}



/*
** An array of this structure defines a list of keywords.
*/
typedef struct PikWord {
  char *zWord;             /* Text of the keyword */
  unsigned char nChar;     /* Length of keyword text in bytes */
  unsigned char eType;     /* Token code */
  unsigned char eCode;     /* Extra code for the token */
  unsigned char eEdge;     /* CP_* code for corner/edge keywords */
} PikWord;

/*
** Keywords
*/
static const PikWord pik_keywords[] = {
  { "above",      5,   T_ABOVE,     0,         0        },
  { "abs",        3,   T_FUNC1,     FN_ABS,    0        },
  { "aligned",    7,   T_ALIGNED,   0,         0        },
  { "and",        3,   T_AND,       0,         0        },
  { "as",         2,   T_AS,        0,         0        },
  { "assert",     6,   T_ASSERT,    0,         0        },
  { "at",         2,   T_AT,        0,         0        },
  { "behind",     6,   T_BEHIND,    0,         0        },
  { "below",      5,   T_BELOW,     0,         0        },
  { "between",    7,   T_BETWEEN,   0,         0        },
  { "big",        3,   T_BIG,       0,         0        },
  { "bold",       4,   T_BOLD,      0,         0        },
  { "bot",        3,   T_EDGEPT,    0,         CP_S     },
  { "bottom",     6,   T_BOTTOM,    0,         CP_S     },
  { "c",          1,   T_EDGEPT,    0,         CP_C     },
  { "ccw",        3,   T_CCW,       0,         0        },
  { "center",     6,   T_CENTER,    0,         CP_C     },
  { "chop",       4,   T_CHOP,      0,         0        },
  { "close",      5,   T_CLOSE,     0,         0        },
  { "color",      5,   T_COLOR,     0,         0        },
  { "cos",        3,   T_FUNC1,     FN_COS,    0        },
  { "cw",         2,   T_CW,        0,         0        },
  { "dashed",     6,   T_DASHED,    0,         0        },
  { "define",     6,   T_DEFINE,    0,         0        },
  { "diameter",   8,   T_DIAMETER,  0,         0        },
  { "dist",       4,   T_DIST,      0,         0        },
  { "dotted",     6,   T_DOTTED,    0,         0        },
  { "down",       4,   T_DOWN,      DIR_DOWN,  0        },
  { "e",          1,   T_EDGEPT,    0,         CP_E     },
  { "east",       4,   T_EDGEPT,    0,         CP_E     },
  { "end",        3,   T_END,       0,         CP_END   },
  { "even",       4,   T_EVEN,      0,         0        },
  { "fill",       4,   T_FILL,      0,         0        },
  { "first",      5,   T_NTH,       0,         0        },
  { "fit",        3,   T_FIT,       0,         0        },
  { "from",       4,   T_FROM,      0,         0        },
  { "go",         2,   T_GO,        0,         0        },
  { "heading",    7,   T_HEADING,   0,         0        },
  { "height",     6,   T_HEIGHT,    0,         0        },
  { "ht",         2,   T_HEIGHT,    0,         0        },
  { "in",         2,   T_IN,        0,         0        },
  { "int",        3,   T_FUNC1,     FN_INT,    0        },
  { "invis",      5,   T_INVIS,     0,         0        },
  { "invisible",  9,   T_INVIS,     0,         0        },
  { "italic",     6,   T_ITALIC,    0,         0        },
  { "last",       4,   T_LAST,      0,         0        },
  { "left",       4,   T_LEFT,      DIR_LEFT,  CP_W     },
  { "ljust",      5,   T_LJUST,     0,         0        },
  { "max",        3,   T_FUNC2,     FN_MAX,    0        },
  { "min",        3,   T_FUNC2,     FN_MIN,    0        },
  { "mono",       4,   T_MONO,      0,         0        },
  { "monospace",  9,   T_MONO,      0,         0        },
  { "n",          1,   T_EDGEPT,    0,         CP_N     },
  { "ne",         2,   T_EDGEPT,    0,         CP_NE    },
  { "north",      5,   T_EDGEPT,    0,         CP_N     },
  { "nw",         2,   T_EDGEPT,    0,         CP_NW    },
  { "of",         2,   T_OF,        0,         0        },
  { "pikchr_date",11,  T_ISODATE,   0,         0,       },
  { "previous",   8,   T_LAST,      0,         0,       },
  { "print",      5,   T_PRINT,     0,         0        },
  { "rad",        3,   T_RADIUS,    0,         0        },
  { "radius",     6,   T_RADIUS,    0,         0        },
  { "right",      5,   T_RIGHT,     DIR_RIGHT, CP_E     },
  { "rjust",      5,   T_RJUST,     0,         0        },
  { "s",          1,   T_EDGEPT,    0,         CP_S     },
  { "same",       4,   T_SAME,      0,         0        },
  { "se",         2,   T_EDGEPT,    0,         CP_SE    },
  { "sin",        3,   T_FUNC1,     FN_SIN,    0        },
  { "small",      5,   T_SMALL,     0,         0        },
  { "solid",      5,   T_SOLID,     0,         0        },
  { "south",      5,   T_EDGEPT,    0,         CP_S     },
  { "sqrt",       4,   T_FUNC1,     FN_SQRT,   0        },
  { "start",      5,   T_START,     0,         CP_START },
  { "sw",         2,   T_EDGEPT,    0,         CP_SW    },
  { "t",          1,   T_TOP,       0,         CP_N     },
  { "the",        3,   T_THE,       0,         0        },
  { "then",       4,   T_THEN,      0,         0        },
  { "thick",      5,   T_THICK,     0,         0        },
  { "thickness",  9,   T_THICKNESS, 0,         0        },
  { "thin",       4,   T_THIN,      0,         0        },
  { "this",       4,   T_THIS,      0,         0        },
  { "to",         2,   T_TO,        0,         0        },
  { "top",        3,   T_TOP,       0,         CP_N     },
  { "until",      5,   T_UNTIL,     0,         0        },
  { "up",         2,   T_UP,        DIR_UP,    0        },
  { "vertex",     6,   T_VERTEX,    0,         0        },
  { "w",          1,   T_EDGEPT,    0,         CP_W     },
  { "way",        3,   T_WAY,       0,         0        },
  { "west",       4,   T_EDGEPT,    0,         CP_W     },
  { "wid",        3,   T_WIDTH,     0,         0        },
  { "width",      5,   T_WIDTH,     0,         0        },
  { "with",       4,   T_WITH,      0,         0        },
  { "x",          1,   T_X,         0,         0        },
  { "y",          1,   T_Y,         0,         0        },
};

/*
** Search a PikWordlist for the given keyword.  Return a pointer to the
** keyword entry found.  Or return 0 if not found.
*/
static const PikWord *pik_find_word(
  const char *zIn,              /* Word to search for */
  int n,                        /* Length of zIn */
  const PikWord *aList,         /* List to search */
  int nList                     /* Number of entries in aList */
){
  int first = 0;
  int last = nList-1;
  while( first<=last ){
    int mid = (first + last)/2;
    int sz = aList[mid].nChar;
    int c = strncmp(zIn, aList[mid].zWord, sz<n ? sz : n);
    if( c==0 ){
      c = n - sz;
      if( c==0 ) return &aList[mid];
    }
    if( c<0 ){
      last = mid-1;
    }else{
      first = mid+1;
    }
  }
  return 0;
}

/*
** Set a symbolic debugger breakpoint on this routine to receive a
** breakpoint when the "#breakpoint" token is parsed.
*/
static void pik_breakpoint(const unsigned char *z){
  /* Prevent C compilers from optimizing out this routine. */
  if( z[2]=='X' ) exit(1);
}


/*
** Return the length of next token.  The token starts on
** the pToken->z character.  Fill in other fields of the
** pToken object as appropriate.
*/
static int pik_token_length(PToken *pToken, int bAllowCodeBlock){
  const unsigned char *z = (const unsigned char*)pToken->z;
  int i;
  unsigned char c, c2;
  switch( z[0] ){
    case '\\': {
      pToken->eType = T_WHITESPACE;
      for(i=1; z[i]=='\r' || z[i]==' ' || z[i]=='\t'; i++){}
      if( z[i]=='\n'  ) return i+1;
      pToken->eType = T_ERROR;
      return 1;
    }
    case ';':
    case '\n': {
      pToken->eType = T_EOL;
      return 1;
    }
    case '"': {
      for(i=1; (c = z[i])!=0; i++){
        if( c=='\\' ){ 
          if( z[i+1]==0 ) break;
          i++;
          continue;
        }
        if( c=='"' ){
          pToken->eType = T_STRING;
          return i+1;
        }
      }
      pToken->eType = T_ERROR;
      return i;
    }
    case ' ':
    case '\t':
    case '\f':
    case '\r': {
      for(i=1; (c = z[i])==' ' || c=='\t' || c=='\r' || c=='\f'; i++){}
      pToken->eType = T_WHITESPACE;
      return i;
    }
    case '#': {
      for(i=1; (c = z[i])!=0 && c!='\n'; i++){}
      pToken->eType = T_WHITESPACE;
      /* If the comment is "#breakpoint" then invoke the pik_breakpoint()
      ** routine.  The pik_breakpoint() routie is a no-op that serves as
      ** a convenient place to set a gdb breakpoint when debugging. */
      if( strncmp((const char*)z,"#breakpoint",11)==0 ) pik_breakpoint(z);
      return i;
    }
    case '/': {
      if( z[1]=='*' ){
        for(i=2; z[i]!=0 && (z[i]!='*' || z[i+1]!='/'); i++){}
        if( z[i]=='*' ){
          pToken->eType = T_WHITESPACE;
          return i+2;
        }else{
          pToken->eType = T_ERROR;
          return i;
        }
      }else if( z[1]=='/' ){
        for(i=2; z[i]!=0 && z[i]!='\n'; i++){}
        pToken->eType = T_WHITESPACE;
        return i;
      }else if( z[1]=='=' ){
        pToken->eType = T_ASSIGN;
        pToken->eCode = T_SLASH;
        return 2;
      }else{
        pToken->eType = T_SLASH;
        return 1;
      }
    }
    case '+': {
      if( z[1]=='=' ){
        pToken->eType = T_ASSIGN;
        pToken->eCode = T_PLUS;
        return 2;
      }
      pToken->eType = T_PLUS;
      return 1;
    }
    case '*': {
      if( z[1]=='=' ){
        pToken->eType = T_ASSIGN;
        pToken->eCode = T_STAR;
        return 2;
      }
      pToken->eType = T_STAR;
      return 1;
    }
    case '%': {   pToken->eType = T_PERCENT; return 1; }
    case '(': {   pToken->eType = T_LP;      return 1; }
    case ')': {   pToken->eType = T_RP;      return 1; }
    case '[': {   pToken->eType = T_LB;      return 1; }
    case ']': {   pToken->eType = T_RB;      return 1; }
    case ',': {   pToken->eType = T_COMMA;   return 1; }
    case ':': {   pToken->eType = T_COLON;   return 1; }
    case '>': {   pToken->eType = T_GT;      return 1; }
    case '=': {
       if( z[1]=='=' ){
         pToken->eType = T_EQ;
         return 2;
       }
       pToken->eType = T_ASSIGN;
       pToken->eCode = T_ASSIGN;
       return 1;
    }
    case '-': {
      if( z[1]=='>' ){
        pToken->eType = T_RARROW;
        return 2;
      }else if( z[1]=='=' ){
        pToken->eType = T_ASSIGN;
        pToken->eCode = T_MINUS;
        return 2;
      }else{
        pToken->eType = T_MINUS;
        return 1;
      }
    }
    case '<': { 
      if( z[1]=='-' ){
         if( z[2]=='>' ){
           pToken->eType = T_LRARROW;
           return 3;
         }else{
           pToken->eType = T_LARROW;
           return 2;
         }
      }else{
        pToken->eType = T_LT;
        return 1;
      }
    }
    case 0xe2: {
      if( z[1]==0x86 ){
        if( z[2]==0x90 ){
          pToken->eType = T_LARROW;   /* <- */
          return 3;
        }
        if( z[2]==0x92 ){
          pToken->eType = T_RARROW;   /* -> */
          return 3;
        }
        if( z[2]==0x94 ){
          pToken->eType = T_LRARROW;  /* <-> */
          return 3;
        }
      }
      pToken->eType = T_ERROR;
      return 1;
    }
    case '{': {
      int len, depth;
      i = 1;
      if( bAllowCodeBlock ){
        depth = 1;
        while( z[i] && depth>0 ){
          PToken x;
          x.z = (char*)(z+i);
          len = pik_token_length(&x, 0);
          if( len==1 ){
            if( z[i]=='{' ) depth++;
            if( z[i]=='}' ) depth--;
          }
          i += len;
        }
      }else{
        depth = 0;
      }
      if( depth ){
        pToken->eType = T_ERROR;
        return 1;
      }
      pToken->eType = T_CODEBLOCK;
      return i;
    }
    case '&': {
      static const struct {
         int nByte;            /* Number of bytes in zEntity */
         int eCode;            /* Corresponding token code */
         const char *zEntity;  /* Name of the HTML entity */
      } aEntity[] = {
                      /*   123456789 1234567 */
         { 6,  T_RARROW,  "&rarr;"           },   /* Same as -> */
         { 12, T_RARROW,  "&rightarrow;"     },   /* Same as -> */
         { 6,  T_LARROW,  "&larr;"           },   /* Same as <- */
         { 11, T_LARROW,  "&leftarrow;"      },   /* Same as <- */
         { 16, T_LRARROW, "&leftrightarrow;" },   /* Same as <-> */
      };
      unsigned int i;
      for(i=0; i<sizeof(aEntity)/sizeof(aEntity[0]); i++){
        if( strncmp((const char*)z,aEntity[i].zEntity,aEntity[i].nByte)==0 ){
          pToken->eType = aEntity[i].eCode;
          return aEntity[i].nByte;
        }
      }
      pToken->eType = T_ERROR;
      return 1;
    }
    default: {
      c = z[0];
      if( c=='.' ){
        unsigned char c1 = z[1];
        if( IsLower(c1) ){
          const PikWord *pFound;
          for(i=2; (c = z[i])>='a' && c<='z'; i++){}
          pFound = pik_find_word((const char*)z+1, i-1,
                                    pik_keywords, count(pik_keywords));
          if( pFound && (pFound->eEdge>0 ||
                         pFound->eType==T_EDGEPT ||
                         pFound->eType==T_START ||
                         pFound->eType==T_END )
          ){
            /* Dot followed by something that is a 2-D place value */
            pToken->eType = T_DOT_E;
          }else if( pFound && (pFound->eType==T_X || pFound->eType==T_Y) ){
            /* Dot followed by "x" or "y" */
            pToken->eType = T_DOT_XY;
          }else{
            /* Any other "dot" */
            pToken->eType = T_DOT_L;
          }
          return 1;
        }else if( IsDigit(c1) ){
          i = 0;
          /* no-op.  Fall through to number handling */
        }else if( IsUpper(c1) ){
          for(i=2; (c = z[i])!=0 && (IsAlnum(c) || c=='_'); i++){}
          pToken->eType = T_DOT_U;
          return 1;
        }else{
          pToken->eType = T_ERROR;
          return 1;
        }
      }
      if( (c>='0' && c<='9') || c=='.' ){
        int nDigit;
        int isInt = 1;
        if( c!='.' ){
          nDigit = 1;
          for(i=1; (c = z[i])>='0' && c<='9'; i++){ nDigit++; }
          if( i==1 && (c=='x' || c=='X') ){
            for(i=2; (c = z[i])!=0 && IsXDigit(c); i++){}
            pToken->eType = T_NUMBER;
            return i;
          }
        }else{
          isInt = 0;
          nDigit = 0;
          i = 0;
        }
        if( c=='.' ){
          isInt = 0;
          for(i++; (c = z[i])>='0' && c<='9'; i++){ nDigit++; }
        }
        if( nDigit==0 ){
          pToken->eType = T_ERROR;
          return i;
        }
        if( c=='e' || c=='E' ){
          int iBefore = i;
          i++;
          c2 = z[i];
          if( c2=='+' || c2=='-' ){
            i++;
            c2 = z[i];
          }
          if( c2<'0' || c>'9' ){
            /* This is not an exp */
            i = iBefore;
          }else{
            i++;
            isInt = 0;
            while( (c = z[i])>='0' && c<='9' ){ i++; }
          }
        }
        c2 = c ? z[i+1] : 0;
        if( isInt ){
          if( (c=='t' && c2=='h')
           || (c=='r' && c2=='d')
           || (c=='n' && c2=='d')
           || (c=='s' && c2=='t')
          ){
            pToken->eType = T_NTH;
            return i+2;
          }
        }
        if( (c=='i' && c2=='n')
         || (c=='c' && c2=='m')
         || (c=='m' && c2=='m')
         || (c=='p' && c2=='t')
         || (c=='p' && c2=='x')
         || (c=='p' && c2=='c')
        ){
          i += 2;
        }
        pToken->eType = T_NUMBER;
        return i;
      }else if( IsLower(c) ){
        const PikWord *pFound;
        for(i=1; (c =  z[i])!=0 && (IsAlnum(c) || c=='_'); i++){}
        pFound = pik_find_word((const char*)z, i,
                               pik_keywords, count(pik_keywords));
        if( pFound ){
          pToken->eType = pFound->eType;
          pToken->eCode = pFound->eCode;
          pToken->eEdge = pFound->eEdge;
          return i;
        }
        pToken->n = i;
        if( pik_find_class(pToken)!=0 ){
          pToken->eType = T_CLASSNAME;
        }else{
          pToken->eType = T_ID;
        }
        return i;
      }else if( c>='A' && c<='Z' ){
        for(i=1; (c =  z[i])!=0 && (IsAlnum(c) || c=='_'); i++){}
        pToken->eType = T_PLACENAME;
        return i;
      }else if( c=='$' && z[1]>='1' && z[1]<='9' && !IsDigit(z[2]) ){
        pToken->eType = T_PARAMETER;
        pToken->eCode = z[1] - '1';
        return 2;
      }else if( c=='_' || c=='$' || c=='@' ){
        for(i=1; (c =  z[i])!=0 && (IsAlnum(c) || c=='_'); i++){}
        pToken->eType = T_ID;
        return i;
      }else{
        pToken->eType = T_ERROR;
        return 1;
      }
    }
  }
}

/*
** Return a pointer to the next non-whitespace token after pThis.
** This is used to help form error messages.
*/
static PToken pik_next_semantic_token(PToken *pThis){
  PToken x;
  int sz;
  int i = pThis->n;
  memset(&x, 0, sizeof(x));
  x.z = pThis->z;
  while(1){
    x.z = pThis->z + i;
    sz = pik_token_length(&x, 1);
    if( x.eType!=T_WHITESPACE ){
      x.n = sz;
      return x;
    }
    i += sz;
  }
}

/* Parser arguments to a macro invocation
**
**     (arg1, arg2, ...)
**
** Arguments are comma-separated, except that commas within string
** literals or with (...), {...}, or [...] do not count.  The argument
** list begins and ends with parentheses.  There can be at most 9
** arguments.
**
** Return the number of bytes in the argument list.
*/
static unsigned int pik_parse_macro_args(
  Pik *p,
  const char *z,     /* Start of the argument list */
  int n,             /* Available bytes */
  PToken *args,      /* Fill in with the arguments */
  PToken *pOuter     /* Arguments of the next outer context, or NULL */
){
  int nArg = 0;
  int i, j, sz;
  int iStart;
  int depth = 0;
  PToken x;
  if( z[0]!='(' ) return 0;
  args[0].z = z+1;
  iStart = 1;
  for(i=1; i<n && z[i]!=')'; i+=sz){
    x.z = z+i;
    sz = pik_token_length(&x, 0);
    if( sz!=1 ) continue;
    if( z[i]==',' && depth<=0 ){
      args[nArg].n = i - iStart;
      if( nArg==8 ){
        x.z = z;
        x.n = 1;
        pik_error(p, &x, "too many macro arguments - max 9");
        return 0;
      }
      nArg++;
      args[nArg].z = z+i+1;
      iStart = i+1;
      depth = 0;
    }else if( z[i]=='(' || z[i]=='{' || z[i]=='[' ){
      depth++;
    }else if( z[i]==')' || z[i]=='}' || z[i]==']' ){
      depth--;
    }
  }
  if( z[i]==')' ){
    args[nArg].n = i - iStart;
    /* Remove leading and trailing whitespace from each argument.
    ** If what remains is one of $1, $2, ... $9 then transfer the
    ** corresponding argument from the outer context */
    for(j=0; j<=nArg; j++){
      PToken *t = &args[j];
      while( t->n>0 && IsSpace(t->z[0]) ){ t->n--; t->z++; }
      while( t->n>0 && IsSpace(t->z[t->n-1]) ){ t->n--; }
      if( t->n==2 && t->z[0]=='$' && t->z[1]>='1' && t->z[1]<='9' ){
        if( pOuter ) *t = pOuter[t->z[1]-'1'];
        else t->n = 0;
      }
    }
    return i+1;
  }
  x.z = z;
  x.n = 1;
  pik_error(p, &x, "unterminated macro argument list");
  return 0;
}

/*
** Split up the content of a PToken into multiple tokens and
** send each to the parser.
*/
void pik_tokenize(Pik *p, PToken *pIn, yyParser *pParser, PToken *aParam){
  unsigned int i;
  int sz = 0;
  PToken token;
  PMacro *pMac;
  for(i=0; i<pIn->n && pIn->z[i] && p->nErr==0; i+=sz){
    token.eCode = 0;
    token.eEdge = 0;
    token.z = pIn->z + i;
    sz = pik_token_length(&token, 1);
    if( token.eType==T_WHITESPACE ){
      /* no-op */
    }else if( sz>50000 ){
      token.n = 1;
      pik_error(p, &token, "token is too long - max length 50000 bytes");
      break;
    }else if( token.eType==T_ERROR ){
      token.n = (unsigned short)(sz & 0xffff);
      pik_error(p, &token, "unrecognized token");
      break;
    }else if( sz+i>pIn->n ){
      token.n = pIn->n - i;
      pik_error(p, &token, "syntax error");
      break;
    }else if( token.eType==T_PARAMETER ){
      /* Substitute a parameter into the input stream */
      if( aParam==0 || aParam[token.eCode].n==0 ){
        continue;
      }
      token.n = (unsigned short)(sz & 0xffff);
      if( p->nCtx>=count(p->aCtx) ){
        pik_error(p, &token, "macros nested too deep");
      }else{
        p->aCtx[p->nCtx++] = token;
        pik_tokenize(p, &aParam[token.eCode], pParser, 0);
        p->nCtx--;
      }
    }else if( token.eType==T_ID
               && (token.n = (unsigned short)(sz & 0xffff), 
                   (pMac = pik_find_macro(p,&token))!=0)
    ){
      PToken args[9];
      unsigned int j = i+sz;
      if( pMac->inUse ){
        pik_error(p, &pMac->macroName, "recursive macro definition");
        break;
      }
      token.n = (short int)(sz & 0xffff);
      if( p->nCtx>=count(p->aCtx) ){
        pik_error(p, &token, "macros nested too deep");
        break;
      } 
      pMac->inUse = 1;
      memset(args, 0, sizeof(args));
      p->aCtx[p->nCtx++] = token;
      sz += pik_parse_macro_args(p, pIn->z+j, pIn->n-j, args, aParam);
      pik_tokenize(p, &pMac->macroBody, pParser, args);
      p->nCtx--;
      pMac->inUse = 0;
    }else{
#if 0
      printf("******** Token %s (%d): \"%.*s\" **************\n",
             yyTokenName[token.eType], token.eType,
             (int)(IsSpace(token.z[0]) ? 0 : sz), token.z);
#endif
      token.n = (unsigned short)(sz & 0xffff);
      if( p->nToken++ > PIKCHR_TOKEN_LIMIT ){
        pik_error(p, &token, "script is too complex");
        break;
      }
      if( token.eType==T_ISODATE ){
        token.z = "\"" MANIFEST_ISODATE "\"";
        token.n = sizeof(MANIFEST_ISODATE)+1;
        token.eType = T_STRING;
      }
      pik_parser(pParser, token.eType, token);
    }
  }
}

/*
** Return the version name.
*/
const char *pikchr_version(void)
  /* Emscripten workaround, else it chokes on the inlined version */;

const char *pikchr_version(void){
  return RELEASE_VERSION " " MANIFEST_ISODATE;
}

/*
** Parse the PIKCHR script contained in zText[].  Return a rendering.  Or
** if an error is encountered, return the error text.  The error message
** is HTML formatted.  So regardless of what happens, the return text
** is safe to be insertd into an HTML output stream.
**
** If pnWidth and pnHeight are not NULL, then this routine writes the
** width and height of the <SVG> object into the integers that they
** point to.  A value of -1 is written if an error is seen.
**
** If zClass is not NULL, then it is a class name to be included in
** the <SVG> markup.
**
** The returned string is contained in memory obtained from malloc()
** and should be released by the caller.
*/
char *pikchr(
  const char *zText,     /* Input PIKCHR source text.  zero-terminated */
  const char *zClass,    /* Add class="%s" to <svg> markup */
  unsigned int mFlags,   /* Flags used to influence rendering behavior */
  int *pnWidth,          /* Write width of <svg> here, if not NULL */
  int *pnHeight          /* Write height here, if not NULL */
){
  Pik s;
  yyParser sParse;

  memset(&s, 0, sizeof(s));
  s.sIn.z = zText;
  s.sIn.n = (unsigned int)strlen(zText);
  s.eDir = DIR_RIGHT;
  s.zClass = zClass;
  s.mFlags = mFlags;
  pik_parserInit(&sParse, &s);
#if 0
  pik_parserTrace(stdout, "parser: ");
#endif
  pik_tokenize(&s, &s.sIn, &sParse, 0);
  if( s.nErr==0 ){
    PToken token;
    memset(&token,0,sizeof(token));
    token.z = zText + (s.sIn.n>0 ? s.sIn.n-1 : 0);
    token.n = 1;
    pik_parser(&sParse, 0, token);
  }
  pik_parserFinalize(&sParse);
  if( s.zOut==0 && s.nErr==0 ){
    pik_append(&s, "<!-- empty pikchr diagram -->\n", -1);
  }
  while( s.pVar ){
    PVar *pNext = s.pVar->pNext;
    free(s.pVar);
    s.pVar = pNext;
  }
  while( s.pMacros ){
    PMacro *pNext = s.pMacros->pNext;
    free(s.pMacros);
    s.pMacros = pNext;
  }
  if( pnWidth ) *pnWidth = s.nErr ? -1 : s.wSVG;
  if( pnHeight ) *pnHeight = s.nErr ? -1 : s.hSVG;
  if( s.zOut ){
    s.zOut[s.nOut] = 0;
    s.zOut = realloc(s.zOut, s.nOut+1);
  }
  return s.zOut;
}

#if defined(PIKCHR_FUZZ)
#include <stdint.h>
int LLVMFuzzerTestOneInput(const uint8_t *aData, size_t nByte){
  int w,h;
  char *zIn, *zOut;
  unsigned int mFlags = nByte & 3;
  zIn = malloc( nByte + 1 );
  if( zIn==0 ) return 0;
  memcpy(zIn, aData, nByte);
  zIn[nByte] = 0;
  zOut = pikchr(zIn, "pikchr", mFlags, &w, &h);
  free(zIn);
  free(zOut);
  return 0;
}
#endif /* PIKCHR_FUZZ */

#if defined(PIKCHR_SHELL)
/* Print a usage comment for the shell and exit. */
static void usage(const char *argv0){
  fprintf(stderr, "usage: %s [OPTIONS] FILE ...\n", argv0);
  fprintf(stderr,
    "Convert Pikchr input files into SVG.  Filename \"-\" means stdin.\n"
    "All output goes to stdout.\n"
    "Options:\n"
    "   --dark-mode      Generate \"dark mode\" output\n"
    "   --dont-stop      Process all files even if earlier files have errors\n"
    "   --svg-only       Emit raw SVG without the HTML wrapper\n"
  );
  exit(1);
}

/* Send text to standard output, but escape HTML markup */
static void print_escape_html(const char *z){
  int j;
  char c;
  while( z[0]!=0 ){
    for(j=0; (c = z[j])!=0 && c!='<' && c!='>' && c!='&'; j++){}
    if( j ) printf("%.*s", j, z);
    z += j+1;
    j = -1;
    if( c=='<' ){
      printf("&lt;");
    }else if( c=='>' ){
      printf("&gt;");
    }else if( c=='&' ){
      printf("&amp;");
    }else if( c==0 ){
      break;
    }
  }
}

/* Read the content of file zFilename into memory obtained from malloc()
** Return the memory.  If something goes wrong (ex: the file does not exist
** or cannot be opened) put an error message on stderr and return NULL.
**
** If the filename is "-" read stdin.
*/
static char *readFile(const char *zFilename){
  FILE *in;
  size_t n;
  size_t nUsed = 0;
  size_t nAlloc = 0;
  char *z = 0, *zNew = 0;
  in = strcmp(zFilename,"-")==0 ? stdin : fopen(zFilename, "rb");
  if( in==0 ){
    fprintf(stderr, "cannot open \"%s\" for reading\n", zFilename);
    return 0;
  }
  while(1){
    if( nUsed+2>=nAlloc ){
      nAlloc = nAlloc*2 + 4000;
      zNew = realloc(z, nAlloc);
    }
    if( zNew==0 ){
      free(z);
      fprintf(stderr, "out of memory trying to allocate %lld bytes\n",
              (long long int)nAlloc);
      exit(1);
    }
    z = zNew;
    n = fread(z+nUsed, 1, nAlloc-nUsed-1, in);
    if( n<=0 ){
      break;
    }
    nUsed += n;
  }
  if( in!=stdin ) fclose(in);
  z[nUsed] = 0;
  return z;
}


/* Testing interface
**
** Generate HTML on standard output that displays both the original
** input text and the rendered SVG for all files named on the command
** line.
*/
int main(int argc, char **argv){
  int i;
  int bSvgOnly = 0;            /* Output SVG only.  No HTML wrapper */
  int bDontStop = 0;           /* Continue in spite of errors */
  int exitCode = 0;            /* What to return */
  int mFlags = 0;              /* mFlags argument to pikchr() */
  const char *zStyle = "";     /* Extra styling */
  const char *zHtmlHdr = 
    "<!DOCTYPE html>\n"
    "<html lang=\"en-US\">\n"
    "<head>\n<title>PIKCHR Test</title>\n"
    "<style>\n"
    "  .hidden {\n"
    "     position: absolute !important;\n"
    "     opacity: 0 !important;\n"
    "     pointer-events: none !important;\n"
    "     display: none !important;\n"
    "  }\n"
    "</style>\n"
    "<script>\n"
    "  function toggleHidden(id){\n"
    "    for(var c of document.getElementById(id).children){\n"
    "      c.classList.toggle('hidden');\n"
    "    }\n"
    "  }\n"
    "</script>\n"
    "<meta charset=\"utf-8\">\n"
    "</head>\n"
    "<body>\n"
  ;
  if( argc<2 ) usage(argv[0]);
  for(i=1; i<argc; i++){
    char *zIn;
    char *zOut;
    int w, h;

    if( argv[i][0]=='-' && argv[i][1]!=0 ){
      char *z = argv[i];
      z++;
      if( z[0]=='-' ) z++;
      if( strcmp(z,"dont-stop")==0 ){
        bDontStop = 1;
      }else
      if( strcmp(z,"dark-mode")==0 ){
        zStyle = "color:white;background-color:black;";
        mFlags |= PIKCHR_DARK_MODE;
      }else
      if( strcmp(z,"svg-only")==0 ){
        if( zHtmlHdr==0 ){
          fprintf(stderr, "the \"%s\" option must come first\n",argv[i]);
          exit(1);
        }
        bSvgOnly = 1;
        mFlags |= PIKCHR_PLAINTEXT_ERRORS;
      }else
      if( strcmp(z,"version")==0 || strcmp(z,"v")==0 ){
        printf("pikchr %s\n", pikchr_version());
        return 0;
      }else
      {
        fprintf(stderr,"unknown option: \"%s\"\n", argv[i]);
        usage(argv[0]);
      }
      continue;
    }
    zIn = readFile(argv[i]);
    if( zIn==0 ) continue;
    zOut = pikchr(zIn, "pikchr", mFlags, &w, &h);
    if( w<0 && !bDontStop ) exitCode = 1;
    if( zOut==0 ){
      fprintf(stderr, "pikchr() returns NULL.  Out of memory?\n");
      if( !bDontStop ) exit(1);
    }else if( bSvgOnly ){
      printf("%s\n", zOut);
    }else{
      if( zHtmlHdr ){
        printf("%s", zHtmlHdr);
        zHtmlHdr = 0;
      }
      printf("<h1>File %s</h1>\n", argv[i]);
      if( w<0 ){
        printf("<p>ERROR</p>\n%s\n", zOut);
      }else{
        printf("<div id=\"svg-%d\" onclick=\"toggleHidden('svg-%d')\">\n",i,i);
        printf("<div style='border:3px solid lightgray;max-width:%dpx;%s'>\n",
               w,zStyle);
        printf("%s</div>\n", zOut);
        printf("<pre class='hidden'>");
        print_escape_html(zIn);
        printf("</pre>\n</div>\n");
      }
    }
    free(zOut);
    free(zIn);
  }
  if( !bSvgOnly ){
    printf("</body></html>\n");
  }
  return exitCode ? EXIT_FAILURE : EXIT_SUCCESS;
}
#endif /* PIKCHR_SHELL */

#ifdef PIKCHR_TCL
#include <tcl.h>
/* Compatability between Tcl8.6 and Tcl9.0 */
#if TCL_MAJOR_VERSION==9
# define CONST const
#elif !defined(Tcl_Size)
  typedef int Tcl_Size;
#endif

/*
** An interface to TCL
**
** TCL command:     pikchr $INPUTTEXT
**
** Returns a list of 3 elements which are the output text, the width, and
** the height.
**
** Register the "pikchr" command by invoking Pikchr_Init(Tcl_Interp*).  Or
** compile this source file as a shared library and load it using the
** "load" command of Tcl.
**
** Compile this source-code file into a shared library using a command
** similar to this:
**
**    gcc -c pikchr.so -DPIKCHR_TCL -shared pikchr.c
*/
static int pik_tcl_command(
  ClientData clientData, /* Not Used */
  Tcl_Interp *interp,    /* The TCL interpreter that invoked this command */
  int objc,              /* Number of arguments */
  Tcl_Obj *CONST objv[]  /* Command arguments */
){
  int w, h;              /* Width and height of the pikchr */
  const char *zIn;       /* Source text input */
  char *zOut;            /* SVG output text */
  Tcl_Obj *pRes;         /* The result TCL object */

  (void)clientData;
  if( objc!=2 ){
    Tcl_WrongNumArgs(interp, 1, objv, "PIKCHR_SOURCE_TEXT");
    return TCL_ERROR;
  }
  zIn = Tcl_GetString(objv[1]);
  w = h = 0;
  zOut = pikchr(zIn, "pikchr", 0, &w, &h);
  if( zOut==0 ){
    return TCL_ERROR;  /* Out of memory */
  }
  pRes = Tcl_NewObj();
  Tcl_ListObjAppendElement(0, pRes, Tcl_NewStringObj(zOut, -1));
  free(zOut);
  Tcl_ListObjAppendElement(0, pRes, Tcl_NewIntObj(w));
  Tcl_ListObjAppendElement(0, pRes, Tcl_NewIntObj(h));
  Tcl_SetObjResult(interp, pRes);
  return TCL_OK;
}

#ifndef PACKAGE_NAME
# define PACKAGE_NAME "pikchr"
#endif
#ifndef PACKAGE_VERSION
# define PACKAGE_VERSION "1.0"
#endif

/* Invoke this routine to register the "pikchr" command with the interpreter
** given in the argument */
int Pikchr_Init(Tcl_Interp *interp){
  Tcl_CreateObjCommand(interp, "pikchr", pik_tcl_command, 0, 0);
  Tcl_PkgProvide(interp, PACKAGE_NAME, PACKAGE_VERSION);
  return TCL_OK;
}


#endif /* PIKCHR_TCL */


} // end %code
