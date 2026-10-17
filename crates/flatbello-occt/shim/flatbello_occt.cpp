// C API shim over Open Cascade. See flatbello_occt.h for the conventions.
//
// Every exported function runs its body through `guard`, which turns any C++ exception into a
// status code and a message for fb_last_error().

#include "flatbello_occt.h"

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepTools.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Tool.hxx>
#include <GCPnts_QuasiUniformDeflection.hxx>
#include <GProp_GProps.hxx>
#include <GeomConvert.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BezierCurve.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <IGESControl_Reader.hxx>
#include <IGESData_IGESModel.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Poly_Triangulation.hxx>
#include <STEPControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColStd_SequenceOfAsciiString.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Circ.hxx>
#include <gp_Elips.hxx>
#include <gp_Pln.hxx>

#ifdef FB_FIXTURES
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <STEPControl_Writer.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <StlAPI_Writer.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColgp_Array1OfPnt.hxx>
#endif

#include <cctype>
#include <exception>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

thread_local std::string g_error;

struct Edge {
    TopoDS_Edge edge;
    bool reversed;
    Handle(Geom_BSplineCurve) bspline; // trimmed copy, filled on demand
};

struct Wire {
    bool is_outer;
    std::vector<Edge> edges;
};

struct FaceCache {
    bool built = false;
    std::vector<Wire> wires;
};

} // namespace

struct fb_shape {
    TopoDS_Shape shape;
    double mm_per_unit = 1.0;
    TopTools_IndexedMapOfShape faces;
    std::vector<FaceCache> cache;
};

namespace {

int fail(int code, const std::string &message) {
    g_error = message;
    return code;
}

/// OCCT prints progress and parse errors to stdout through its default messenger; a library
/// must not, and failures are reported through fb_last_error instead.
void silence_occt() {
    static std::once_flag once;
    std::call_once(once, [] { Message::DefaultMessenger()->ChangePrinters().Clear(); });
}

template <typename F> int guard(F &&body) {
    try {
        g_error.clear();
        silence_occt();
        return body();
    } catch (const Standard_Failure &e) {
        const char *msg = e.GetMessageString();
        return fail(FB_EOCCT, std::string(e.DynamicType()->Name()) + ": " + (msg ? msg : ""));
    } catch (const std::exception &e) {
        return fail(FB_EUNKNOWN, e.what());
    } catch (...) {
        return fail(FB_EUNKNOWN, "unknown C++ exception");
    }
}

void put(double out[3], const gp_XYZ &v) {
    out[0] = v.X();
    out[1] = v.Y();
    out[2] = v.Z();
}

/// Millimetres per unit for a STEP/IGES length unit name, or 0 if unknown.
double unit_scale(std::string name) {
    for (auto &c : name) {
        c = static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
    }
    if (name.find("MILLI") != std::string::npos || name == "MM") return 1.0;
    if (name.find("CENTI") != std::string::npos || name == "CM") return 10.0;
    if (name.find("MICRO") != std::string::npos || name == "UM") return 0.001;
    if (name.find("KILO") != std::string::npos || name == "KM") return 1.0e6;
    if (name.find("INCH") != std::string::npos || name == "IN") return 25.4;
    if (name.find("FOOT") != std::string::npos || name.find("FEET") != std::string::npos ||
        name == "FT")
        return 304.8;
    if (name.find("METRE") != std::string::npos || name.find("METER") != std::string::npos ||
        name == "M")
        return 1000.0;
    return 0.0;
}

fb_shape *wrap(const TopoDS_Shape &s, double mm_per_unit) {
    auto *out = new fb_shape();
    out->shape = s;
    out->mm_per_unit = mm_per_unit > 0.0 ? mm_per_unit : 1.0;
    TopExp::MapShapes(s, TopAbs_FACE, out->faces);
    out->cache.resize(static_cast<size_t>(out->faces.Extent()));
    return out;
}

/// Writes bytes to a fresh temporary file and returns its path.
std::filesystem::path temp_file(const uint8_t *data, size_t len, const char *ext) {
    std::random_device rd;
    auto path = std::filesystem::temp_directory_path() /
                ("flatbello-" + std::to_string(rd()) + std::to_string(rd()) + ext);
    std::ofstream f(path, std::ios::binary);
    f.write(reinterpret_cast<const char *>(data), static_cast<std::streamsize>(len));
    if (!f) throw std::runtime_error("cannot write temporary file " + path.string());
    return path;
}

int checked_face(fb_shape *s, size_t face, TopoDS_Face &out) {
    if (!s || face >= s->cache.size()) return fail(FB_EINVAL, "face index out of range");
    out = TopoDS::Face(s->faces.FindKey(static_cast<int>(face) + 1));
    return FB_OK;
}

/// Fills the wire/edge cache of one face.
int build_face(fb_shape *s, size_t face) {
    TopoDS_Face f;
    if (int rc = checked_face(s, face, f)) return rc;
    FaceCache &c = s->cache[face];
    if (c.built) return FB_OK;
    TopoDS_Wire outer = BRepTools::OuterWire(f);
    for (TopExp_Explorer wx(f, TopAbs_WIRE); wx.More(); wx.Next()) {
        TopoDS_Wire w = TopoDS::Wire(wx.Current());
        Wire wire;
        wire.is_outer = !outer.IsNull() && w.IsSame(outer);
        for (BRepTools_WireExplorer ex(w, f); ex.More(); ex.Next()) {
            wire.edges.push_back(Edge{ex.Current(), ex.Orientation() == TopAbs_REVERSED, {}});
        }
        c.wires.push_back(std::move(wire));
    }
    c.built = true;
    return FB_OK;
}

int checked_edge(fb_shape *s, size_t face, size_t wire, size_t edge, Edge *&out) {
    if (int rc = build_face(s, face)) return rc;
    FaceCache &c = s->cache[face];
    if (wire >= c.wires.size()) return fail(FB_EINVAL, "wire index out of range");
    if (edge >= c.wires[wire].edges.size()) return fail(FB_EINVAL, "edge index out of range");
    out = &c.wires[wire].edges[edge];
    return FB_OK;
}

/// The edge's B-spline (converted from a Bézier if needed), trimmed to the edge's range and
/// made non-periodic so its knots are clamped.
Handle(Geom_BSplineCurve) trimmed_bspline(Edge &e) {
    if (!e.bspline.IsNull()) return e.bspline;
    BRepAdaptor_Curve c(e.edge);
    Handle(Geom_BSplineCurve) bs;
    if (c.GetType() == GeomAbs_BezierCurve) {
        bs = GeomConvert::CurveToBSplineCurve(c.Bezier());
    } else {
        bs = Handle(Geom_BSplineCurve)::DownCast(c.BSpline()->Copy());
    }
    if (bs.IsNull()) throw std::runtime_error("edge is not a B-spline");
    if (bs->IsPeriodic()) bs->SetNotPeriodic();
    const double first = c.FirstParameter(), last = c.LastParameter();
    const double eps = Precision::PConfusion();
    if (first > bs->FirstParameter() + eps || last < bs->LastParameter() - eps) {
        bs->Segment(first, last);
    }
    e.bspline = bs;
    return bs;
}

} // namespace

extern "C" {

const char *fb_occt_version(void) { return OCC_VERSION_COMPLETE; }

const char *fb_last_error(void) { return g_error.c_str(); }

int fb_load_step(const uint8_t *data, size_t len, fb_shape **out) {
    return guard([&]() -> int {
        if (!data || !out) return fail(FB_EINVAL, "null argument");
        *out = nullptr;
        STEPControl_Reader reader;
#if OCC_VERSION_HEX >= 0x070700
        std::string buffer(reinterpret_cast<const char *>(data), len);
        std::istringstream in(buffer);
        const IFSelect_ReturnStatus read = reader.ReadStream("flatbello.step", in);
#else
        // Older versions read only from files.
        auto path = temp_file(data, len, ".step");
        struct Remove {
            std::filesystem::path p;
            ~Remove() {
                std::error_code ec;
                std::filesystem::remove(p, ec);
            }
        } remove{path};
        const IFSelect_ReturnStatus read = reader.ReadFile(path.string().c_str());
#endif
        if (read != IFSelect_RetDone) {
            return fail(FB_EREAD, "not a readable STEP file");
        }
        TColStd_SequenceOfAsciiString lengths, angles, solid_angles;
        reader.FileUnits(lengths, angles, solid_angles);
        double scale = lengths.Length() > 0 ? unit_scale(lengths.First().ToCString()) : 1.0;
        if (reader.TransferRoots() <= 0) return fail(FB_EEMPTY, "the STEP file has no shapes");
        TopoDS_Shape s = reader.OneShape();
        if (s.IsNull()) return fail(FB_EEMPTY, "the STEP file has no shapes");
        *out = wrap(s, scale);
        return FB_OK;
    });
}

int fb_load_iges(const uint8_t *data, size_t len, fb_shape **out) {
    return guard([&]() -> int {
        if (!data || !out) return fail(FB_EINVAL, "null argument");
        *out = nullptr;
        // IGESControl_Reader reads only from files.
        auto path = temp_file(data, len, ".igs");
        struct Remove {
            std::filesystem::path p;
            ~Remove() {
                std::error_code ec;
                std::filesystem::remove(p, ec);
            }
        } remove{path};
        IGESControl_Reader reader;
        if (reader.ReadFile(path.string().c_str()) != IFSelect_RetDone) {
            return fail(FB_EREAD, "not a readable IGES file");
        }
        // The global section gives the file's unit in metres.
        double scale = reader.IGESModel()->GlobalSection().UnitValue() * 1000.0;
        if (reader.TransferRoots() <= 0) return fail(FB_EEMPTY, "the IGES file has no shapes");
        TopoDS_Shape s = reader.OneShape();
        if (s.IsNull()) return fail(FB_EEMPTY, "the IGES file has no shapes");
        *out = wrap(s, scale);
        return FB_OK;
    });
}

void fb_shape_free(fb_shape *shape) {
    try {
        delete shape;
    } catch (...) {
    }
}

int fb_shape_file_unit(const fb_shape *shape, double *mm_per_unit) {
    if (!shape || !mm_per_unit) return fail(FB_EINVAL, "null argument");
    *mm_per_unit = shape->mm_per_unit;
    return FB_OK;
}

int fb_face_count(fb_shape *shape, size_t *count) {
    if (!shape || !count) return fail(FB_EINVAL, "null argument");
    *count = shape->cache.size();
    return FB_OK;
}

int fb_face_info_get(fb_shape *shape, size_t face, fb_face_info *out) {
    return guard([&]() -> int {
        if (!out) return fail(FB_EINVAL, "null argument");
        TopoDS_Face f;
        if (int rc = checked_face(shape, face, f)) return rc;
        *out = fb_face_info{};
        out->reversed = f.Orientation() == TopAbs_REVERSED;
        BRepAdaptor_Surface surf(f, Standard_True);
        switch (surf.GetType()) {
        case GeomAbs_Plane: {
            out->surface = FB_SURFACE_PLANE;
            const gp_Ax3 pos = surf.Plane().Position();
            // The surface normal is X × Y, which is −Direction for a left-handed placement.
            gp_XYZ n = pos.XDirection().XYZ().Crossed(pos.YDirection().XYZ());
            if (out->reversed) n.Reverse();
            put(out->origin, pos.Location().XYZ());
            put(out->normal, n);
            put(out->x_dir, pos.XDirection().XYZ());
            break;
        }
        case GeomAbs_Cylinder:
            out->surface = FB_SURFACE_CYLINDER;
            break;
        default:
            out->surface = FB_SURFACE_OTHER;
            break;
        }
        GProp_GProps props;
        BRepGProp::SurfaceProperties(f, props);
        out->area = props.Mass();
        if (int rc = build_face(shape, face)) return rc;
        out->wire_count = shape->cache[face].wires.size();
        return FB_OK;
    });
}

int fb_wire_info_get(fb_shape *shape, size_t face, size_t wire, fb_wire_info *out) {
    return guard([&]() -> int {
        if (!out) return fail(FB_EINVAL, "null argument");
        if (int rc = build_face(shape, face)) return rc;
        const FaceCache &c = shape->cache[face];
        if (wire >= c.wires.size()) return fail(FB_EINVAL, "wire index out of range");
        out->is_outer = c.wires[wire].is_outer;
        out->edge_count = c.wires[wire].edges.size();
        return FB_OK;
    });
}

int fb_edge_info_get(fb_shape *shape, size_t face, size_t wire, size_t edge, fb_edge_info *out) {
    return guard([&]() -> int {
        if (!out) return fail(FB_EINVAL, "null argument");
        Edge *e = nullptr;
        if (int rc = checked_edge(shape, face, wire, edge, e)) return rc;
        *out = fb_edge_info{};
        out->reversed = e->reversed;
        if (BRep_Tool::Degenerated(e->edge)) {
            out->degenerated = 1;
            out->curve = FB_CURVE_OTHER;
            return FB_OK;
        }
        BRepAdaptor_Curve c(e->edge);
        out->first = c.FirstParameter();
        out->last = c.LastParameter();
        put(out->start, c.Value(out->first).XYZ());
        put(out->end, c.Value(out->last).XYZ());
        switch (c.GetType()) {
        case GeomAbs_Line:
            out->curve = FB_CURVE_LINE;
            break;
        case GeomAbs_Circle: {
            out->curve = FB_CURVE_CIRCLE;
            const gp_Circ circ = c.Circle();
            put(out->center, circ.Location().XYZ());
            put(out->axis, circ.Axis().Direction().XYZ());
            put(out->x_dir, circ.XAxis().Direction().XYZ());
            out->radius1 = circ.Radius();
            break;
        }
        case GeomAbs_Ellipse: {
            out->curve = FB_CURVE_ELLIPSE;
            const gp_Elips el = c.Ellipse();
            put(out->center, el.Location().XYZ());
            put(out->axis, el.Axis().Direction().XYZ());
            put(out->x_dir, el.XAxis().Direction().XYZ());
            out->radius1 = el.MajorRadius();
            out->radius2 = el.MinorRadius();
            break;
        }
        case GeomAbs_BSplineCurve:
        case GeomAbs_BezierCurve: {
            out->curve = FB_CURVE_BSPLINE;
            Handle(Geom_BSplineCurve) bs = trimmed_bspline(*e);
            out->degree = static_cast<size_t>(bs->Degree());
            out->pole_count = static_cast<size_t>(bs->NbPoles());
            out->knot_count = static_cast<size_t>(bs->NbPoles() + bs->Degree() + 1);
            out->rational = bs->IsRational();
            break;
        }
        default:
            out->curve = FB_CURVE_OTHER;
            break;
        }
        return FB_OK;
    });
}

int fb_edge_bspline(fb_shape *shape, size_t face, size_t wire, size_t edge, double *poles,
                    size_t pole_cap, double *weights, double *knots, size_t knot_cap) {
    return guard([&]() -> int {
        Edge *e = nullptr;
        if (int rc = checked_edge(shape, face, wire, edge, e)) return rc;
        Handle(Geom_BSplineCurve) bs = trimmed_bspline(*e);
        const int n = bs->NbPoles();
        const int m = n + bs->Degree() + 1;
        if (!poles || !knots || pole_cap < static_cast<size_t>(n) ||
            knot_cap < static_cast<size_t>(m)) {
            return fail(FB_ESIZE, "B-spline buffers too small");
        }
        for (int i = 1; i <= n; ++i) {
            put(poles + 3 * (i - 1), bs->Pole(i).XYZ());
            if (weights) weights[i - 1] = bs->Weight(i);
        }
        TColStd_Array1OfReal seq(1, m);
        bs->KnotSequence(seq);
        for (int i = 1; i <= m; ++i) knots[i - 1] = seq(i);
        return FB_OK;
    });
}

int fb_edge_sample(fb_shape *shape, size_t face, size_t wire, size_t edge, double deflection,
                   double *points, size_t cap, size_t *count) {
    return guard([&]() -> int {
        if (!count || !(deflection > 0.0)) return fail(FB_EINVAL, "bad argument");
        Edge *e = nullptr;
        if (int rc = checked_edge(shape, face, wire, edge, e)) return rc;
        BRepAdaptor_Curve c(e->edge);
        GCPnts_QuasiUniformDeflection sampler(c, deflection, c.FirstParameter(),
                                              c.LastParameter());
        if (!sampler.IsDone()) return fail(FB_EOCCT, "edge sampling failed");
        const int n = sampler.NbPoints();
        *count = static_cast<size_t>(n);
        if (!points || cap < static_cast<size_t>(n)) return fail(FB_ESIZE, "buffer too small");
        for (int i = 1; i <= n; ++i) put(points + 3 * (i - 1), sampler.Value(i).XYZ());
        return FB_OK;
    });
}

int fb_tessellate(fb_shape *shape, double deflection) {
    return guard([&]() -> int {
        if (!shape || !(deflection > 0.0)) return fail(FB_EINVAL, "bad argument");
        BRepMesh_IncrementalMesh mesher(shape->shape, deflection, Standard_False, 0.5,
                                        Standard_False);
        if (!mesher.IsDone()) return fail(FB_EOCCT, "meshing failed");
        return FB_OK;
    });
}

int fb_face_mesh(fb_shape *shape, size_t face, double *nodes, size_t node_cap, size_t *node_count,
                 uint32_t *triangles, size_t tri_cap, size_t *tri_count) {
    return guard([&]() -> int {
        if (!node_count || !tri_count) return fail(FB_EINVAL, "null argument");
        TopoDS_Face f;
        if (int rc = checked_face(shape, face, f)) return rc;
        TopLoc_Location loc;
        Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(f, loc);
        if (tri.IsNull()) {
            *node_count = 0;
            *tri_count = 0;
            return FB_OK;
        }
        const int nn = tri->NbNodes(), nt = tri->NbTriangles();
        *node_count = static_cast<size_t>(nn);
        *tri_count = static_cast<size_t>(nt);
        if (!nodes || !triangles || node_cap < *node_count || tri_cap < *tri_count) {
            return fail(FB_ESIZE, "buffer too small");
        }
        const gp_Trsf trsf = loc.Transformation();
        for (int i = 1; i <= nn; ++i) put(nodes + 3 * (i - 1), tri->Node(i).Transformed(trsf).XYZ());
        const bool reversed = f.Orientation() == TopAbs_REVERSED;
        for (int i = 1; i <= nt; ++i) {
            int a, b, c;
            tri->Triangle(i).Get(a, b, c);
            if (reversed) std::swap(b, c);
            uint32_t *t = triangles + 3 * (i - 1);
            t[0] = static_cast<uint32_t>(a - 1);
            t[1] = static_cast<uint32_t>(b - 1);
            t[2] = static_cast<uint32_t>(c - 1);
        }
        return FB_OK;
    });
}

#ifdef FB_FIXTURES

namespace {

int unified(const TopoDS_Shape &s, fb_shape **out) {
    ShapeUpgrade_UnifySameDomain unify(s, Standard_True, Standard_True, Standard_False);
    unify.Build();
    *out = wrap(unify.Shape(), 1.0);
    return FB_OK;
}

} // namespace

int fb_make_box(double x, double y, double z, double dx, double dy, double dz, fb_shape **out) {
    return guard([&]() -> int {
        if (!out) return fail(FB_EINVAL, "null argument");
        *out = wrap(BRepPrimAPI_MakeBox(gp_Pnt(x, y, z), dx, dy, dz).Shape(), 1.0);
        return FB_OK;
    });
}

int fb_make_cylinder(const double base[3], const double axis[3], double radius, double height,
                     fb_shape **out) {
    return guard([&]() -> int {
        if (!out || !base || !axis) return fail(FB_EINVAL, "null argument");
        gp_Ax2 ax(gp_Pnt(base[0], base[1], base[2]), gp_Dir(axis[0], axis[1], axis[2]));
        *out = wrap(BRepPrimAPI_MakeCylinder(ax, radius, height).Shape(), 1.0);
        return FB_OK;
    });
}

int fb_make_bspline_prism(const double *poles, size_t n, size_t degree, double height,
                          fb_shape **out) {
    return guard([&]() -> int {
        if (!out || !poles || degree < 1 || n < degree + 1) return fail(FB_EINVAL, "bad argument");
        const int np = static_cast<int>(n), deg = static_cast<int>(degree);
        TColgp_Array1OfPnt pts(1, np);
        for (int i = 1; i <= np; ++i) {
            const double *p = poles + 3 * (i - 1);
            pts(i) = gp_Pnt(p[0], p[1], p[2]);
        }
        // Clamped uniform knots: n - degree + 1 distinct values, end multiplicity degree + 1.
        const int nk = np - deg + 1;
        TColStd_Array1OfReal knots(1, nk);
        TColStd_Array1OfInteger mults(1, nk);
        for (int i = 1; i <= nk; ++i) {
            knots(i) = static_cast<double>(i - 1);
            mults(i) = (i == 1 || i == nk) ? deg + 1 : 1;
        }
        Handle(Geom_BSplineCurve) curve = new Geom_BSplineCurve(pts, knots, mults, deg);
        TopoDS_Edge e1 = BRepBuilderAPI_MakeEdge(curve).Edge();
        TopoDS_Edge e2 = BRepBuilderAPI_MakeEdge(pts(np), pts(1)).Edge();
        TopoDS_Wire w = BRepBuilderAPI_MakeWire(e1, e2).Wire();
        TopoDS_Face f = BRepBuilderAPI_MakeFace(w, Standard_True).Face();
        *out = wrap(BRepPrimAPI_MakePrism(f, gp_Vec(0.0, 0.0, height)).Shape(), 1.0);
        return FB_OK;
    });
}

int fb_fuse(const fb_shape *a, const fb_shape *b, fb_shape **out) {
    return guard([&]() -> int {
        if (!a || !b || !out) return fail(FB_EINVAL, "null argument");
        BRepAlgoAPI_Fuse op(a->shape, b->shape);
        if (!op.IsDone() || op.HasErrors()) return fail(FB_EOCCT, "fuse failed");
        return unified(op.Shape(), out);
    });
}

int fb_cut(const fb_shape *a, const fb_shape *b, fb_shape **out) {
    return guard([&]() -> int {
        if (!a || !b || !out) return fail(FB_EINVAL, "null argument");
        BRepAlgoAPI_Cut op(a->shape, b->shape);
        if (!op.IsDone() || op.HasErrors()) return fail(FB_EOCCT, "cut failed");
        return unified(op.Shape(), out);
    });
}

int fb_write_step(const fb_shape *shape, const char *path) {
    return guard([&]() -> int {
        if (!shape || !path) return fail(FB_EINVAL, "null argument");
        STEPControl_Writer writer;
        if (writer.Transfer(shape->shape, STEPControl_AsIs) != IFSelect_RetDone) {
            return fail(FB_EOCCT, "STEP transfer failed");
        }
        if (writer.Write(path) != IFSelect_RetDone) return fail(FB_EREAD, "STEP write failed");
        return FB_OK;
    });
}

int fb_write_stl(fb_shape *shape, const char *path, double deflection) {
    return guard([&]() -> int {
        if (!shape || !path || !(deflection > 0.0)) return fail(FB_EINVAL, "bad argument");
        BRepMesh_IncrementalMesh mesher(shape->shape, deflection, Standard_False, 0.5,
                                        Standard_False);
        StlAPI_Writer writer;
        if (!writer.Write(shape->shape, path)) return fail(FB_EREAD, "STL write failed");
        return FB_OK;
    });
}

#endif

} // extern "C"
