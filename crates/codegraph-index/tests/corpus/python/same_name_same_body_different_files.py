# Golden corpus fixture — pins B2-ID-001 (SymbolSnapshotId must never
# collide for two distinct symbols sharing the same name AND the same
# signature body, when they live in DIFFERENT files). This file is
# paired with same_name_same_body_different_files_b.py; both define a
# method named init_weights with byte-identical bodies, minimized from
# the real B2-ID-001 repro (two PyTorch model classes, RNNModel and
# TransformerModel, each with an init_weights method whose body was
# textually identical).
class ModelA:
    def init_weights(self):
        initrange = 0.1
        self.encoder.weight = initrange
