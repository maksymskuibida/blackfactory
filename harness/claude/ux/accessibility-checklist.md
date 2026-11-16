# Accessibility checklist

The verifier settles the accessibility result with a sentence that starts `accessibility:` and names what was checked, against the standard in force (the brief names it; WCAG 2.2 level AA when nothing else is set). Written new for this harness against WCAG 2.2; see `NOTICE.md`.

Check each item that applies to the interface kind. An item that cannot be checked with the driver's tools is reported as unreachable, never as met.

## Perceivable

- Every image, icon and control that carries meaning has a text alternative or an accessible name.
- Text contrast is at least 4.5 to 1 (3 to 1 for large text and interface parts).
- Nothing is told by colour alone.
- Text can be enlarged to 200 percent without losing content.
- Terminal output carries its meaning in words, not only in colour or symbols.

## Operable

- Every action works from the keyboard alone, in a sensible order, with no trap.
- The focus is always visible.
- Touch targets are at least 24 by 24 points.
- Nothing flashes more than three times in a second.
- Page, screen or command has a clear title or heading.

## Understandable

- Each field has a label, and an error says what is wrong and how to fix it.
- Navigation and names are consistent across screens.
- An action with lasting effect can be undone or confirmed.

## Robust

- Controls expose a name, a role and a state to assistive technology (the accessibility tree on the web and mobile).
- The page or screen has no duplicated IDs and valid structure.
- Status changes are announced, not only shown.
