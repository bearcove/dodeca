# The Intended Scope And Purpose Of Pikchr

Pikchr is a specialized tool designed for one specific purpose:

  *  Pikchr generates diagrams for technical documentation written in
     Markdown or similar markup languages using an enduring language that
     is easy for humans to read and maintain using a generic text
     editor.

To this end, Pikchr diagrams are designed to be:

  *  Cross platform &rarr; Pikchr is not tied to any particular
     computer architecture or operating system.  The current implementation
     is a single file of generic C code using no external resources apart
     from the standard C library.

  *  Simple, well-defined, and easy-to-learn syntax.

  *  Enduring &rarr; The diagram source text should be easily readable,
     editable, and understandable by people not yet born.  Pikchr is based
     on the [PIC][1] language, which was developed in the early 1980s.

[1]: https://en.wikipedia.org/wiki/Pic_language

What Pikchr is <u>not</u>:

  *  Pikchr is not intended for marketing graphics.  Pikchr
     strives to present information in a dry and mathematical style.
     The objective of Pikchr is to convey truth, not feeling.

  *  Pikchr is not intended for generating charts and graphs.  It could
     perhaps be used for this.  One might propose extensions to make it more
     suitable for this.  But that is not its current purpose.

  *  Pikchr is not intended to generate CAD/CAM images.  The rendered
     diagrams are close enough to display concepts, but do not have the
     pixel-perfect precision required for CAD/CAM.

  *  Pikchr is not intended as a replacement for point-and-click diagrams
     creation software.  Pikchr is to point-and-click systems as
     Markdown is to MS-Word or Google-Docs.  Point-and-click interfaces
     have their place.  But so do text-based systems such as Markdown and
     Pikchr.
