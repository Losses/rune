pub const HANNING_MULTIPLIERS: [f64; 2048] = [
	0.0000023508,
	0.0000094032,
	0.000021157,
	0.000037612,
	0.000058769,
	0.000084626,
	0.00011518,
	0.00015044,
	0.0001904,
	0.00023506,
	0.00028442,
	0.00033848,
	0.00039723,
	0.00046069,
	0.00052884,
	0.00060168,
	0.00067923,
	0.00076147,
	0.0008484,
	0.00094003,
	0.0010363,
	0.0011374,
	0.0012431,
	0.0013535,
	0.0014685,
	0.0015883,
	0.0017128,
	0.0018419,
	0.0019757,
	0.0021142,
	0.0022574,
	0.0024053,
	0.0025578,
	0.0027151,
	0.002877,
	0.0030435,
	0.0032148,
	0.0033907,
	0.0035713,
	0.0037566,
	0.0039465,
	0.0041411,
	0.0043403,
	0.0045443,
	0.0047528,
	0.0049661,
	0.0051839,
	0.0054065,
	0.0056337,
	0.0058655,
	0.006102,
	0.0063431,
	0.0065889,
	0.0068393,
	0.0070943,
	0.007354,
	0.0076183,
	0.0078873,
	0.0081608,
	0.008439,
	0.0087219,
	0.0090093,
	0.0093013,
	0.009598,
	0.0098993,
	0.010205,
	0.010516,
	0.010831,
	0.01115,
	0.011475,
	0.011804,
	0.012137,
	0.012475,
	0.012818,
	0.013165,
	0.013517,
	0.013873,
	0.014234,
	0.0146,
	0.01497,
	0.015344,
	0.015724,
	0.016107,
	0.016496,
	0.016889,
	0.017286,
	0.017688,
	0.018094,
	0.018505,
	0.018921,
	0.019341,
	0.019766,
	0.020195,
	0.020628,
	0.021066,
	0.021509,
	0.021956,
	0.022408,
	0.022864,
	0.023324,
	0.023789,
	0.024259,
	0.024733,
	0.025211,
	0.025694,
	0.026182,
	0.026674,
	0.02717,
	0.027671,
	0.028176,
	0.028686,
	0.0292,
	0.029718,
	0.030241,
	0.030768,
	0.0313,
	0.031836,
	0.032377,
	0.032922,
	0.033471,
	0.034025,
	0.034583,
	0.035146,
	0.035712,
	0.036284,
	0.036859,
	0.037439,
	0.038024,
	0.038612,
	0.039205,
	0.039803,
	0.040404,
	0.04101,
	0.04162,
	0.042235,
	0.042854,
	0.043477,
	0.044105,
	0.044736,
	0.045372,
	0.046013,
	0.046657,
	0.047306,
	0.047959,
	0.048617,
	0.049278,
	0.049944,
	0.050614,
	0.051288,
	0.051967,
	0.05265,
	0.053337,
	0.054028,
	0.054723,
	0.055423,
	0.056126,
	0.056834,
	0.057546,
	0.058263,
	0.058983,
	0.059707,
	0.060436,
	0.061169,
	0.061906,
	0.062647,
	0.063392,
	0.064141,
	0.064895,
	0.065652,
	0.066413,
	0.067179,
	0.067949,
	0.068722,
	0.0695,
	0.070282,
	0.071068,
	0.071858,
	0.072652,
	0.07345,
	0.074252,
	0.075058,
	0.075868,
	0.076682,
	0.0775,
	0.078321,
	0.079147,
	0.079977,
	0.080811,
	0.081649,
	0.08249,
	0.083336,
	0.084185,
	0.085039,
	0.085896,
	0.086757,
	0.087622,
	0.088491,
	0.089364,
	0.090241,
	0.091121,
	0.092006,
	0.092894,
	0.093786,
	0.094682,
	0.095582,
	0.096485,
	0.097392,
	0.098303,
	0.099218,
	0.10014,
	0.10106,
	0.10199,
	0.10292,
	0.10385,
	0.10479,
	0.10573,
	0.10667,
	0.10762,
	0.10857,
	0.10953,
	0.11049,
	0.11145,
	0.11242,
	0.11339,
	0.11436,
	0.11534,
	0.11632,
	0.11731,
	0.1183,
	0.11929,
	0.12028,
	0.12128,
	0.12228,
	0.12329,
	0.1243,
	0.12531,
	0.12633,
	0.12735,
	0.12838,
	0.1294,
	0.13043,
	0.13147,
	0.13251,
	0.13355,
	0.13459,
	0.13564,
	0.13669,
	0.13775,
	0.13881,
	0.13987,
	0.14093,
	0.142,
	0.14307,
	0.14415,
	0.14523,
	0.14631,
	0.1474,
	0.14849,
	0.14958,
	0.15067,
	0.15177,
	0.15287,
	0.15398,
	0.15509,
	0.1562,
	0.15731,
	0.15843,
	0.15955,
	0.16068,
	0.1618,
	0.16294,
	0.16407,
	0.16521,
	0.16635,
	0.16749,
	0.16864,
	0.16979,
	0.17094,
	0.1721,
	0.17325,
	0.17442,
	0.17558,
	0.17675,
	0.17792,
	0.1791,
	0.18027,
	0.18145,
	0.18264,
	0.18382,
	0.18501,
	0.1862,
	0.1874,
	0.1886,
	0.1898,
	0.191,
	0.19221,
	0.19342,
	0.19463,
	0.19585,
	0.19707,
	0.19829,
	0.19951,
	0.20074,
	0.20197,
	0.2032,
	0.20444,
	0.20567,
	0.20691,
	0.20816,
	0.2094,
	0.21065,
	0.2119,
	0.21316,
	0.21442,
	0.21568,
	0.21694,
	0.2182,
	0.21947,
	0.22074,
	0.22202,
	0.22329,
	0.22457,
	0.22585,
	0.22713,
	0.22842,
	0.22971,
	0.231,
	0.23229,
	0.23359,
	0.23489,
	0.23619,
	0.23749,
	0.2388,
	0.24011,
	0.24142,
	0.24273,
	0.24405,
	0.24537,
	0.24669,
	0.24801,
	0.24934,
	0.25066,
	0.25199,
	0.25333,
	0.25466,
	0.256,
	0.25734,
	0.25868,
	0.26002,
	0.26137,
	0.26272,
	0.26407,
	0.26542,
	0.26678,
	0.26813,
	0.26949,
	0.27086,
	0.27222,
	0.27359,
	0.27495,
	0.27632,
	0.2777,
	0.27907,
	0.28045,
	0.28183,
	0.28321,
	0.28459,
	0.28597,
	0.28736,
	0.28875,
	0.29014,
	0.29153,
	0.29293,
	0.29432,
	0.29572,
	0.29712,
	0.29852,
	0.29993,
	0.30133,
	0.30274,
	0.30415,
	0.30556,
	0.30698,
	0.30839,
	0.30981,
	0.31123,
	0.31265,
	0.31407,
	0.3155,
	0.31692,
	0.31835,
	0.31978,
	0.32121,
	0.32264,
	0.32408,
	0.32551,
	0.32695,
	0.32839,
	0.32983,
	0.33127,
	0.33272,
	0.33416,
	0.33561,
	0.33706,
	0.33851,
	0.33996,
	0.34141,
	0.34287,
	0.34433,
	0.34578,
	0.34724,
	0.3487,
	0.35017,
	0.35163,
	0.35309,
	0.35456,
	0.35603,
	0.3575,
	0.35897,
	0.36044,
	0.36191,
	0.36339,
	0.36486,
	0.36634,
	0.36782,
	0.3693,
	0.37078,
	0.37226,
	0.37374,
	0.37522,
	0.37671,
	0.3782,
	0.37968,
	0.38117,
	0.38266,
	0.38415,
	0.38565,
	0.38714,
	0.38863,
	0.39013,
	0.39162,
	0.39312,
	0.39462,
	0.39612,
	0.39762,
	0.39912,
	0.40062,
	0.40213,
	0.40363,
	0.40513,
	0.40664,
	0.40815,
	0.40965,
	0.41116,
	0.41267,
	0.41418,
	0.41569,
	0.41721,
	0.41872,
	0.42023,
	0.42174,
	0.42326,
	0.42478,
	0.42629,
	0.42781,
	0.42933,
	0.43084,
	0.43236,
	0.43388,
	0.4354,
	0.43692,
	0.43844,
	0.43997,
	0.44149,
	0.44301,
	0.44453,
	0.44606,
	0.44758,
	0.44911,
	0.45063,
	0.45216,
	0.45369,
	0.45521,
	0.45674,
	0.45827,
	0.4598,
	0.46132,
	0.46285,
	0.46438,
	0.46591,
	0.46744,
	0.46897,
	0.4705,
	0.47203,
	0.47356,
	0.4751,
	0.47663,
	0.47816,
	0.47969,
	0.48122,
	0.48275,
	0.48429,
	0.48582,
	0.48735,
	0.48888,
	0.49042,
	0.49195,
	0.49348,
	0.49502,
	0.49655,
	0.49808,
	0.49962,
	0.50115,
	0.50268,
	0.50422,
	0.50575,
	0.50728,
	0.50882,
	0.51035,
	0.51188,
	0.51341,
	0.51495,
	0.51648,
	0.51801,
	0.51954,
	0.52108,
	0.52261,
	0.52414,
	0.52567,
	0.5272,
	0.52873,
	0.53026,
	0.53179,
	0.53332,
	0.53485,
	0.53638,
	0.53791,
	0.53944,
	0.54097,
	0.5425,
	0.54402,
	0.54555,
	0.54708,
	0.5486,
	0.55013,
	0.55165,
	0.55318,
	0.5547,
	0.55623,
	0.55775,
	0.55927,
	0.5608,
	0.56232,
	0.56384,
	0.56536,
	0.56688,
	0.5684,
	0.56992,
	0.57143,
	0.57295,
	0.57447,
	0.57598,
	0.5775,
	0.57901,
	0.58053,
	0.58204,
	0.58355,
	0.58506,
	0.58657,
	0.58808,
	0.58959,
	0.5911,
	0.59261,
	0.59411,
	0.59562,
	0.59712,
	0.59863,
	0.60013,
	0.60163,
	0.60313,
	0.60463,
	0.60613,
	0.60763,
	0.60912,
	0.61062,
	0.61211,
	0.61361,
	0.6151,
	0.61659,
	0.61808,
	0.61957,
	0.62106,
	0.62255,
	0.62403,
	0.62552,
	0.627,
	0.62848,
	0.62996,
	0.63144,
	0.63292,
	0.6344,
	0.63588,
	0.63735,
	0.63883,
	0.6403,
	0.64177,
	0.64324,
	0.64471,
	0.64617,
	0.64764,
	0.6491,
	0.65057,
	0.65203,
	0.65349,
	0.65495,
	0.6564,
	0.65786,
	0.65931,
	0.66077,
	0.66222,
	0.66367,
	0.66511,
	0.66656,
	0.668,
	0.66945,
	0.67089,
	0.67233,
	0.67377,
	0.67521,
	0.67664,
	0.67807,
	0.67951,
	0.68094,
	0.68236,
	0.68379,
	0.68522,
	0.68664,
	0.68806,
	0.68948,
	0.6909,
	0.69232,
	0.69373,
	0.69514,
	0.69655,
	0.69796,
	0.69937,
	0.70077,
	0.70218,
	0.70358,
	0.70498,
	0.70638,
	0.70777,
	0.70916,
	0.71056,
	0.71195,
	0.71333,
	0.71472,
	0.7161,
	0.71748,
	0.71886,
	0.72024,
	0.72162,
	0.72299,
	0.72436,
	0.72573,
	0.7271,
	0.72846,
	0.72983,
	0.73119,
	0.73254,
	0.7339,
	0.73525,
	0.73661,
	0.73796,
	0.7393,
	0.74065,
	0.74199,
	0.74333,
	0.74467,
	0.74601,
	0.74734,
	0.74867,
	0.75,
	0.75133,
	0.75265,
	0.75397,
	0.75529,
	0.75661,
	0.75792,
	0.75924,
	0.76055,
	0.76185,
	0.76316,
	0.76446,
	0.76576,
	0.76706,
	0.76835,
	0.76965,
	0.77094,
	0.77222,
	0.77351,
	0.77479,
	0.77607,
	0.77735,
	0.77862,
	0.77989,
	0.78116,
	0.78243,
	0.78369,
	0.78495,
	0.78621,
	0.78747,
	0.78872,
	0.78997,
	0.79122,
	0.79246,
	0.79371,
	0.79495,
	0.79618,
	0.79742,
	0.79865,
	0.79988,
	0.8011,
	0.80232,
	0.80354,
	0.80476,
	0.80597,
	0.80719,
	0.80839,
	0.8096,
	0.8108,
	0.812,
	0.8132,
	0.81439,
	0.81558,
	0.81677,
	0.81796,
	0.81914,
	0.82032,
	0.82149,
	0.82266,
	0.82383,
	0.825,
	0.82616,
	0.82732,
	0.82848,
	0.82964,
	0.83079,
	0.83194,
	0.83308,
	0.83422,
	0.83536,
	0.8365,
	0.83763,
	0.83876,
	0.83989,
	0.84101,
	0.84213,
	0.84324,
	0.84436,
	0.84547,
	0.84657,
	0.84768,
	0.84878,
	0.84988,
	0.85097,
	0.85206,
	0.85315,
	0.85423,
	0.85531,
	0.85639,
	0.85746,
	0.85853,
	0.8596,
	0.86066,
	0.86172,
	0.86278,
	0.86383,
	0.86488,
	0.86593,
	0.86697,
	0.86801,
	0.86905,
	0.87008,
	0.87111,
	0.87214,
	0.87316,
	0.87418,
	0.87519,
	0.8762,
	0.87721,
	0.87822,
	0.87922,
	0.88022,
	0.88121,
	0.8822,
	0.88319,
	0.88417,
	0.88515,
	0.88613,
	0.8871,
	0.88807,
	0.88903,
	0.88999,
	0.89095,
	0.8919,
	0.89285,
	0.8938,
	0.89474,
	0.89568,
	0.89662,
	0.89755,
	0.89848,
	0.8994,
	0.90032,
	0.90124,
	0.90215,
	0.90306,
	0.90397,
	0.90487,
	0.90577,
	0.90666,
	0.90755,
	0.90844,
	0.90932,
	0.9102,
	0.91107,
	0.91194,
	0.91281,
	0.91367,
	0.91453,
	0.91539,
	0.91624,
	0.91709,
	0.91793,
	0.91877,
	0.91961,
	0.92044,
	0.92127,
	0.92209,
	0.92291,
	0.92373,
	0.92454,
	0.92535,
	0.92615,
	0.92695,
	0.92775,
	0.92854,
	0.92933,
	0.93011,
	0.93089,
	0.93166,
	0.93244,
	0.9332,
	0.93397,
	0.93473,
	0.93548,
	0.93623,
	0.93698,
	0.93772,
	0.93846,
	0.9392,
	0.93993,
	0.94066,
	0.94138,
	0.9421,
	0.94281,
	0.94352,
	0.94423,
	0.94493,
	0.94563,
	0.94632,
	0.94701,
	0.94769,
	0.94837,
	0.94905,
	0.94972,
	0.95039,
	0.95105,
	0.95171,
	0.95237,
	0.95302,
	0.95367,
	0.95431,
	0.95495,
	0.95558,
	0.95621,
	0.95684,
	0.95746,
	0.95807,
	0.95869,
	0.95929,
	0.9599,
	0.9605,
	0.96109,
	0.96168,
	0.96227,
	0.96285,
	0.96343,
	0.964,
	0.96457,
	0.96514,
	0.9657,
	0.96625,
	0.9668,
	0.96735,
	0.96789,
	0.96843,
	0.96897,
	0.9695,
	0.97002,
	0.97054,
	0.97106,
	0.97157,
	0.97208,
	0.97258,
	0.97308,
	0.97357,
	0.97406,
	0.97455,
	0.97503,
	0.9755,
	0.97598,
	0.97644,
	0.97691,
	0.97736,
	0.97782,
	0.97827,
	0.97871,
	0.97915,
	0.97959,
	0.98002,
	0.98045,
	0.98087,
	0.98129,
	0.9817,
	0.98211,
	0.98251,
	0.98291,
	0.98331,
	0.9837,
	0.98409,
	0.98447,
	0.98484,
	0.98522,
	0.98558,
	0.98595,
	0.98631,
	0.98666,
	0.98701,
	0.98735,
	0.98769,
	0.98803,
	0.98836,
	0.98869,
	0.98901,
	0.98933,
	0.98964,
	0.98995,
	0.99025,
	0.99055,
	0.99085,
	0.99114,
	0.99142,
	0.9917,
	0.99198,
	0.99225,
	0.99251,
	0.99278,
	0.99303,
	0.99329,
	0.99353,
	0.99378,
	0.99402,
	0.99425,
	0.99448,
	0.99471,
	0.99493,
	0.99514,
	0.99535,
	0.99556,
	0.99576,
	0.99596,
	0.99615,
	0.99634,
	0.99652,
	0.9967,
	0.99687,
	0.99704,
	0.9972,
	0.99736,
	0.99752,
	0.99767,
	0.99781,
	0.99796,
	0.99809,
	0.99822,
	0.99835,
	0.99847,
	0.99859,
	0.9987,
	0.99881,
	0.99891,
	0.99901,
	0.99911,
	0.9992,
	0.99928,
	0.99936,
	0.99944,
	0.99951,
	0.99957,
	0.99963,
	0.99969,
	0.99974,
	0.99979,
	0.99983,
	0.99987,
	0.9999,
	0.99993,
	0.99995,
	0.99997,
	0.99999,
	0.99999,
	1.0,
	1.0,
	0.99999,
	0.99999,
	0.99997,
	0.99995,
	0.99993,
	0.9999,
	0.99987,
	0.99983,
	0.99979,
	0.99974,
	0.99969,
	0.99963,
	0.99957,
	0.99951,
	0.99944,
	0.99936,
	0.99928,
	0.9992,
	0.99911,
	0.99901,
	0.99891,
	0.99881,
	0.9987,
	0.99859,
	0.99847,
	0.99835,
	0.99822,
	0.99809,
	0.99796,
	0.99781,
	0.99767,
	0.99752,
	0.99736,
	0.9972,
	0.99704,
	0.99687,
	0.9967,
	0.99652,
	0.99634,
	0.99615,
	0.99596,
	0.99576,
	0.99556,
	0.99535,
	0.99514,
	0.99493,
	0.99471,
	0.99448,
	0.99425,
	0.99402,
	0.99378,
	0.99353,
	0.99329,
	0.99303,
	0.99278,
	0.99251,
	0.99225,
	0.99198,
	0.9917,
	0.99142,
	0.99114,
	0.99085,
	0.99055,
	0.99025,
	0.98995,
	0.98964,
	0.98933,
	0.98901,
	0.98869,
	0.98836,
	0.98803,
	0.98769,
	0.98735,
	0.98701,
	0.98666,
	0.98631,
	0.98595,
	0.98558,
	0.98522,
	0.98484,
	0.98447,
	0.98409,
	0.9837,
	0.98331,
	0.98291,
	0.98251,
	0.98211,
	0.9817,
	0.98129,
	0.98087,
	0.98045,
	0.98002,
	0.97959,
	0.97915,
	0.97871,
	0.97827,
	0.97782,
	0.97736,
	0.97691,
	0.97644,
	0.97598,
	0.9755,
	0.97503,
	0.97455,
	0.97406,
	0.97357,
	0.97308,
	0.97258,
	0.97208,
	0.97157,
	0.97106,
	0.97054,
	0.97002,
	0.9695,
	0.96897,
	0.96843,
	0.96789,
	0.96735,
	0.9668,
	0.96625,
	0.9657,
	0.96514,
	0.96457,
	0.964,
	0.96343,
	0.96285,
	0.96227,
	0.96168,
	0.96109,
	0.9605,
	0.9599,
	0.95929,
	0.95869,
	0.95807,
	0.95746,
	0.95684,
	0.95621,
	0.95558,
	0.95495,
	0.95431,
	0.95367,
	0.95302,
	0.95237,
	0.95171,
	0.95105,
	0.95039,
	0.94972,
	0.94905,
	0.94837,
	0.94769,
	0.94701,
	0.94632,
	0.94563,
	0.94493,
	0.94423,
	0.94352,
	0.94281,
	0.9421,
	0.94138,
	0.94066,
	0.93993,
	0.9392,
	0.93846,
	0.93772,
	0.93698,
	0.93623,
	0.93548,
	0.93473,
	0.93397,
	0.9332,
	0.93244,
	0.93166,
	0.93089,
	0.93011,
	0.92933,
	0.92854,
	0.92775,
	0.92695,
	0.92615,
	0.92535,
	0.92454,
	0.92373,
	0.92291,
	0.92209,
	0.92127,
	0.92044,
	0.91961,
	0.91877,
	0.91793,
	0.91709,
	0.91624,
	0.91539,
	0.91453,
	0.91367,
	0.91281,
	0.91194,
	0.91107,
	0.9102,
	0.90932,
	0.90844,
	0.90755,
	0.90666,
	0.90577,
	0.90487,
	0.90397,
	0.90306,
	0.90215,
	0.90124,
	0.90032,
	0.8994,
	0.89848,
	0.89755,
	0.89662,
	0.89568,
	0.89474,
	0.8938,
	0.89285,
	0.8919,
	0.89095,
	0.88999,
	0.88903,
	0.88807,
	0.8871,
	0.88613,
	0.88515,
	0.88417,
	0.88319,
	0.8822,
	0.88121,
	0.88022,
	0.87922,
	0.87822,
	0.87721,
	0.8762,
	0.87519,
	0.87418,
	0.87316,
	0.87214,
	0.87111,
	0.87008,
	0.86905,
	0.86801,
	0.86697,
	0.86593,
	0.86488,
	0.86383,
	0.86278,
	0.86172,
	0.86066,
	0.8596,
	0.85853,
	0.85746,
	0.85639,
	0.85531,
	0.85423,
	0.85315,
	0.85206,
	0.85097,
	0.84988,
	0.84878,
	0.84768,
	0.84657,
	0.84547,
	0.84436,
	0.84324,
	0.84213,
	0.84101,
	0.83989,
	0.83876,
	0.83763,
	0.8365,
	0.83536,
	0.83422,
	0.83308,
	0.83194,
	0.83079,
	0.82964,
	0.82848,
	0.82732,
	0.82616,
	0.825,
	0.82383,
	0.82266,
	0.82149,
	0.82032,
	0.81914,
	0.81796,
	0.81677,
	0.81558,
	0.81439,
	0.8132,
	0.812,
	0.8108,
	0.8096,
	0.80839,
	0.80719,
	0.80597,
	0.80476,
	0.80354,
	0.80232,
	0.8011,
	0.79988,
	0.79865,
	0.79742,
	0.79618,
	0.79495,
	0.79371,
	0.79246,
	0.79122,
	0.78997,
	0.78872,
	0.78747,
	0.78621,
	0.78495,
	0.78369,
	0.78243,
	0.78116,
	0.77989,
	0.77862,
	0.77735,
	0.77607,
	0.77479,
	0.77351,
	0.77222,
	0.77094,
	0.76965,
	0.76835,
	0.76706,
	0.76576,
	0.76446,
	0.76316,
	0.76185,
	0.76055,
	0.75924,
	0.75792,
	0.75661,
	0.75529,
	0.75397,
	0.75265,
	0.75133,
	0.75,
	0.74867,
	0.74734,
	0.74601,
	0.74467,
	0.74333,
	0.74199,
	0.74065,
	0.7393,
	0.73796,
	0.73661,
	0.73525,
	0.7339,
	0.73254,
	0.73119,
	0.72983,
	0.72846,
	0.7271,
	0.72573,
	0.72436,
	0.72299,
	0.72162,
	0.72024,
	0.71886,
	0.71748,
	0.7161,
	0.71472,
	0.71333,
	0.71195,
	0.71056,
	0.70916,
	0.70777,
	0.70638,
	0.70498,
	0.70358,
	0.70218,
	0.70077,
	0.69937,
	0.69796,
	0.69655,
	0.69514,
	0.69373,
	0.69232,
	0.6909,
	0.68948,
	0.68806,
	0.68664,
	0.68522,
	0.68379,
	0.68236,
	0.68094,
	0.67951,
	0.67807,
	0.67664,
	0.67521,
	0.67377,
	0.67233,
	0.67089,
	0.66945,
	0.668,
	0.66656,
	0.66511,
	0.66367,
	0.66222,
	0.66077,
	0.65931,
	0.65786,
	0.6564,
	0.65495,
	0.65349,
	0.65203,
	0.65057,
	0.6491,
	0.64764,
	0.64617,
	0.64471,
	0.64324,
	0.64177,
	0.6403,
	0.63883,
	0.63735,
	0.63588,
	0.6344,
	0.63292,
	0.63144,
	0.62996,
	0.62848,
	0.627,
	0.62552,
	0.62403,
	0.62255,
	0.62106,
	0.61957,
	0.61808,
	0.61659,
	0.6151,
	0.61361,
	0.61211,
	0.61062,
	0.60912,
	0.60763,
	0.60613,
	0.60463,
	0.60313,
	0.60163,
	0.60013,
	0.59863,
	0.59712,
	0.59562,
	0.59411,
	0.59261,
	0.5911,
	0.58959,
	0.58808,
	0.58657,
	0.58506,
	0.58355,
	0.58204,
	0.58053,
	0.57901,
	0.5775,
	0.57598,
	0.57447,
	0.57295,
	0.57143,
	0.56992,
	0.5684,
	0.56688,
	0.56536,
	0.56384,
	0.56232,
	0.5608,
	0.55927,
	0.55775,
	0.55623,
	0.5547,
	0.55318,
	0.55165,
	0.55013,
	0.5486,
	0.54708,
	0.54555,
	0.54402,
	0.5425,
	0.54097,
	0.53944,
	0.53791,
	0.53638,
	0.53485,
	0.53332,
	0.53179,
	0.53026,
	0.52873,
	0.5272,
	0.52567,
	0.52414,
	0.52261,
	0.52108,
	0.51954,
	0.51801,
	0.51648,
	0.51495,
	0.51341,
	0.51188,
	0.51035,
	0.50882,
	0.50728,
	0.50575,
	0.50422,
	0.50268,
	0.50115,
	0.49962,
	0.49808,
	0.49655,
	0.49502,
	0.49348,
	0.49195,
	0.49042,
	0.48888,
	0.48735,
	0.48582,
	0.48429,
	0.48275,
	0.48122,
	0.47969,
	0.47816,
	0.47663,
	0.4751,
	0.47356,
	0.47203,
	0.4705,
	0.46897,
	0.46744,
	0.46591,
	0.46438,
	0.46285,
	0.46132,
	0.4598,
	0.45827,
	0.45674,
	0.45521,
	0.45369,
	0.45216,
	0.45063,
	0.44911,
	0.44758,
	0.44606,
	0.44453,
	0.44301,
	0.44149,
	0.43997,
	0.43844,
	0.43692,
	0.4354,
	0.43388,
	0.43236,
	0.43084,
	0.42933,
	0.42781,
	0.42629,
	0.42478,
	0.42326,
	0.42174,
	0.42023,
	0.41872,
	0.41721,
	0.41569,
	0.41418,
	0.41267,
	0.41116,
	0.40965,
	0.40815,
	0.40664,
	0.40513,
	0.40363,
	0.40213,
	0.40062,
	0.39912,
	0.39762,
	0.39612,
	0.39462,
	0.39312,
	0.39162,
	0.39013,
	0.38863,
	0.38714,
	0.38565,
	0.38415,
	0.38266,
	0.38117,
	0.37968,
	0.3782,
	0.37671,
	0.37522,
	0.37374,
	0.37226,
	0.37078,
	0.3693,
	0.36782,
	0.36634,
	0.36486,
	0.36339,
	0.36191,
	0.36044,
	0.35897,
	0.3575,
	0.35603,
	0.35456,
	0.35309,
	0.35163,
	0.35017,
	0.3487,
	0.34724,
	0.34578,
	0.34433,
	0.34287,
	0.34141,
	0.33996,
	0.33851,
	0.33706,
	0.33561,
	0.33416,
	0.33272,
	0.33127,
	0.32983,
	0.32839,
	0.32695,
	0.32551,
	0.32408,
	0.32264,
	0.32121,
	0.31978,
	0.31835,
	0.31692,
	0.3155,
	0.31407,
	0.31265,
	0.31123,
	0.30981,
	0.30839,
	0.30698,
	0.30556,
	0.30415,
	0.30274,
	0.30133,
	0.29993,
	0.29852,
	0.29712,
	0.29572,
	0.29432,
	0.29293,
	0.29153,
	0.29014,
	0.28875,
	0.28736,
	0.28597,
	0.28459,
	0.28321,
	0.28183,
	0.28045,
	0.27907,
	0.2777,
	0.27632,
	0.27495,
	0.27359,
	0.27222,
	0.27086,
	0.26949,
	0.26813,
	0.26678,
	0.26542,
	0.26407,
	0.26272,
	0.26137,
	0.26002,
	0.25868,
	0.25734,
	0.256,
	0.25466,
	0.25333,
	0.25199,
	0.25066,
	0.24934,
	0.24801,
	0.24669,
	0.24537,
	0.24405,
	0.24273,
	0.24142,
	0.24011,
	0.2388,
	0.23749,
	0.23619,
	0.23489,
	0.23359,
	0.23229,
	0.231,
	0.22971,
	0.22842,
	0.22713,
	0.22585,
	0.22457,
	0.22329,
	0.22202,
	0.22074,
	0.21947,
	0.2182,
	0.21694,
	0.21568,
	0.21442,
	0.21316,
	0.2119,
	0.21065,
	0.2094,
	0.20816,
	0.20691,
	0.20567,
	0.20444,
	0.2032,
	0.20197,
	0.20074,
	0.19951,
	0.19829,
	0.19707,
	0.19585,
	0.19463,
	0.19342,
	0.19221,
	0.191,
	0.1898,
	0.1886,
	0.1874,
	0.1862,
	0.18501,
	0.18382,
	0.18264,
	0.18145,
	0.18027,
	0.1791,
	0.17792,
	0.17675,
	0.17558,
	0.17442,
	0.17325,
	0.1721,
	0.17094,
	0.16979,
	0.16864,
	0.16749,
	0.16635,
	0.16521,
	0.16407,
	0.16294,
	0.1618,
	0.16068,
	0.15955,
	0.15843,
	0.15731,
	0.1562,
	0.15509,
	0.15398,
	0.15287,
	0.15177,
	0.15067,
	0.14958,
	0.14849,
	0.1474,
	0.14631,
	0.14523,
	0.14415,
	0.14307,
	0.142,
	0.14093,
	0.13987,
	0.13881,
	0.13775,
	0.13669,
	0.13564,
	0.13459,
	0.13355,
	0.13251,
	0.13147,
	0.13043,
	0.1294,
	0.12838,
	0.12735,
	0.12633,
	0.12531,
	0.1243,
	0.12329,
	0.12228,
	0.12128,
	0.12028,
	0.11929,
	0.1183,
	0.11731,
	0.11632,
	0.11534,
	0.11436,
	0.11339,
	0.11242,
	0.11145,
	0.11049,
	0.10953,
	0.10857,
	0.10762,
	0.10667,
	0.10573,
	0.10479,
	0.10385,
	0.10292,
	0.10199,
	0.10106,
	0.10014,
	0.099218,
	0.098303,
	0.097392,
	0.096485,
	0.095582,
	0.094682,
	0.093786,
	0.092894,
	0.092006,
	0.091121,
	0.090241,
	0.089364,
	0.088491,
	0.087622,
	0.086757,
	0.085896,
	0.085039,
	0.084185,
	0.083336,
	0.08249,
	0.081649,
	0.080811,
	0.079977,
	0.079147,
	0.078321,
	0.0775,
	0.076682,
	0.075868,
	0.075058,
	0.074252,
	0.07345,
	0.072652,
	0.071858,
	0.071068,
	0.070282,
	0.0695,
	0.068722,
	0.067949,
	0.067179,
	0.066413,
	0.065652,
	0.064895,
	0.064141,
	0.063392,
	0.062647,
	0.061906,
	0.061169,
	0.060436,
	0.059707,
	0.058983,
	0.058263,
	0.057546,
	0.056834,
	0.056126,
	0.055423,
	0.054723,
	0.054028,
	0.053337,
	0.05265,
	0.051967,
	0.051288,
	0.050614,
	0.049944,
	0.049278,
	0.048617,
	0.047959,
	0.047306,
	0.046657,
	0.046013,
	0.045372,
	0.044736,
	0.044105,
	0.043477,
	0.042854,
	0.042235,
	0.04162,
	0.04101,
	0.040404,
	0.039803,
	0.039205,
	0.038612,
	0.038024,
	0.037439,
	0.036859,
	0.036284,
	0.035712,
	0.035146,
	0.034583,
	0.034025,
	0.033471,
	0.032922,
	0.032377,
	0.031836,
	0.0313,
	0.030768,
	0.030241,
	0.029718,
	0.0292,
	0.028686,
	0.028176,
	0.027671,
	0.02717,
	0.026674,
	0.026182,
	0.025694,
	0.025211,
	0.024733,
	0.024259,
	0.023789,
	0.023324,
	0.022864,
	0.022408,
	0.021956,
	0.021509,
	0.021066,
	0.020628,
	0.020195,
	0.019766,
	0.019341,
	0.018921,
	0.018505,
	0.018094,
	0.017688,
	0.017286,
	0.016889,
	0.016496,
	0.016107,
	0.015724,
	0.015344,
	0.01497,
	0.0146,
	0.014234,
	0.013873,
	0.013517,
	0.013165,
	0.012818,
	0.012475,
	0.012137,
	0.011804,
	0.011475,
	0.01115,
	0.010831,
	0.010516,
	0.010205,
	0.0098993,
	0.009598,
	0.0093013,
	0.0090093,
	0.0087219,
	0.008439,
	0.0081608,
	0.0078873,
	0.0076183,
	0.007354,
	0.0070943,
	0.0068393,
	0.0065889,
	0.0063431,
	0.006102,
	0.0058655,
	0.0056337,
	0.0054065,
	0.0051839,
	0.0049661,
	0.0047528,
	0.0045443,
	0.0043403,
	0.0041411,
	0.0039465,
	0.0037566,
	0.0035713,
	0.0033907,
	0.0032148,
	0.0030435,
	0.002877,
	0.0027151,
	0.0025578,
	0.0024053,
	0.0022574,
	0.0021142,
	0.0019757,
	0.0018419,
	0.0017128,
	0.0015883,
	0.0014685,
	0.0013535,
	0.0012431,
	0.0011374,
	0.0010363,
	0.00094003,
	0.0008484,
	0.00076147,
	0.00067923,
	0.00060168,
	0.00052884,
	0.00046069,
	0.00039723,
	0.00033848,
	0.00028442,
	0.00023506,
	0.0001904,
	0.00015044,
	0.00011518,
	0.000084626,
	0.000058769,
	0.000037612,
	0.000021157,
	0.0000094032,
	0.0000023508,
];