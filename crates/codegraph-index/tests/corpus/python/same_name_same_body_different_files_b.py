# Second half of the B2-ID-001 pin — see the sibling file's own
# comment. Byte-identical method body, different class, different
# file: SymbolSnapshotId must differ (span + file path both differ).
class ModelB:
    def init_weights(self):
        initrange = 0.1
        self.encoder.weight = initrange
