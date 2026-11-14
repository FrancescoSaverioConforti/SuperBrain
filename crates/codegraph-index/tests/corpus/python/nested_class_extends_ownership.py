# Golden corpus fixture — pins B2-ID-007 (nested-declaration ownership
# boundary). `Inner`'s own base clause (`Base`) must be attributed to
# `Inner` alone, never also to `Outer`, which merely textually contains
# it. Minimized from B2-ID-007's own real regression test fixture.
class Base:
    pass


class Outer:
    class Inner(Base):
        pass
