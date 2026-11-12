// Golden corpus fixture -- Semantic Resolution & Reliability Checkpoint,
// Priority 2 (SignatureTooLong -> ExtractionRejected linkage). A real
// extraction rejection (the method signature below genuinely exceeds
// codegraph_index::extract::candidate::MAX_SIGNATURE_BYTES, 4096 bytes,
// via parameter count, not a contrived string) must produce a
// CandidateRejection::SignatureTooLong carrying this method's own name,
// which a caller (resolve_calls_with_extraction_rejections) can then
// link to a same-named, zero-candidate call site as
// UnresolvedReason::ExtractionRejected -- never collapsing to the same
// Unknown reason a genuinely external target would get. This is a
// permanent regression fixture for the real-corpus finding this
// checkpoint measured against App-vNext/Polly (213 real
// SignatureTooLong rejections on that corpus).
namespace SignatureTooLongLinkage;

public class Caller
{
    public void InvokesRejectedTarget()
    {
        VeryLongSignatureMethod();
    }
}

public class Target
{
    public static void VeryLongSignatureMethod(int p0, int p1, int p2, int p3, int p4, int p5, int p6, int p7, int p8, int p9, int p10, int p11, int p12, int p13, int p14, int p15, int p16, int p17, int p18, int p19, int p20, int p21, int p22, int p23, int p24, int p25, int p26, int p27, int p28, int p29, int p30, int p31, int p32, int p33, int p34, int p35, int p36, int p37, int p38, int p39, int p40, int p41, int p42, int p43, int p44, int p45, int p46, int p47, int p48, int p49, int p50, int p51, int p52, int p53, int p54, int p55, int p56, int p57, int p58, int p59, int p60, int p61, int p62, int p63, int p64, int p65, int p66, int p67, int p68, int p69, int p70, int p71, int p72, int p73, int p74, int p75, int p76, int p77, int p78, int p79, int p80, int p81, int p82, int p83, int p84, int p85, int p86, int p87, int p88, int p89, int p90, int p91, int p92, int p93, int p94, int p95, int p96, int p97, int p98, int p99, int p100, int p101, int p102, int p103, int p104, int p105, int p106, int p107, int p108, int p109, int p110, int p111, int p112, int p113, int p114, int p115, int p116, int p117, int p118, int p119, int p120, int p121, int p122, int p123, int p124, int p125, int p126, int p127, int p128, int p129, int p130, int p131, int p132, int p133, int p134, int p135, int p136, int p137, int p138, int p139, int p140, int p141, int p142, int p143, int p144, int p145, int p146, int p147, int p148, int p149, int p150, int p151, int p152, int p153, int p154, int p155, int p156, int p157, int p158, int p159, int p160, int p161, int p162, int p163, int p164, int p165, int p166, int p167, int p168, int p169, int p170, int p171, int p172, int p173, int p174, int p175, int p176, int p177, int p178, int p179, int p180, int p181, int p182, int p183, int p184, int p185, int p186, int p187, int p188, int p189, int p190, int p191, int p192, int p193, int p194, int p195, int p196, int p197, int p198, int p199, int p200, int p201, int p202, int p203, int p204, int p205, int p206, int p207, int p208, int p209, int p210, int p211, int p212, int p213, int p214, int p215, int p216, int p217, int p218, int p219, int p220, int p221, int p222, int p223, int p224, int p225, int p226, int p227, int p228, int p229, int p230, int p231, int p232, int p233, int p234, int p235, int p236, int p237, int p238, int p239, int p240, int p241, int p242, int p243, int p244, int p245, int p246, int p247, int p248, int p249, int p250, int p251, int p252, int p253, int p254, int p255, int p256, int p257, int p258, int p259, int p260, int p261, int p262, int p263, int p264, int p265, int p266, int p267, int p268, int p269, int p270, int p271, int p272, int p273, int p274, int p275, int p276, int p277, int p278, int p279, int p280, int p281, int p282, int p283, int p284, int p285, int p286, int p287, int p288, int p289, int p290, int p291, int p292, int p293, int p294, int p295, int p296, int p297, int p298, int p299, int p300, int p301, int p302, int p303, int p304, int p305, int p306, int p307, int p308, int p309, int p310, int p311, int p312, int p313, int p314, int p315, int p316, int p317, int p318, int p319, int p320, int p321, int p322, int p323, int p324, int p325, int p326, int p327, int p328, int p329, int p330, int p331, int p332, int p333, int p334, int p335, int p336, int p337, int p338, int p339, int p340, int p341, int p342, int p343, int p344, int p345, int p346, int p347, int p348, int p349, int p350, int p351, int p352, int p353, int p354, int p355, int p356, int p357, int p358, int p359, int p360, int p361, int p362, int p363, int p364, int p365, int p366, int p367, int p368, int p369, int p370, int p371, int p372, int p373, int p374, int p375, int p376, int p377, int p378, int p379, int p380, int p381, int p382, int p383, int p384, int p385, int p386, int p387, int p388, int p389, int p390, int p391, int p392, int p393, int p394, int p395, int p396, int p397, int p398, int p399, int p400, int p401, int p402, int p403, int p404, int p405, int p406, int p407, int p408, int p409, int p410, int p411, int p412, int p413, int p414, int p415, int p416, int p417, int p418, int p419, int p420, int p421, int p422, int p423, int p424, int p425, int p426, int p427, int p428, int p429, int p430, int p431, int p432, int p433, int p434, int p435, int p436, int p437, int p438, int p439, int p440, int p441, int p442, int p443, int p444, int p445, int p446, int p447, int p448, int p449)
    {
    }
}
