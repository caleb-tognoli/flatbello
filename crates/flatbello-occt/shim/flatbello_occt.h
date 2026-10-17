/* C API over Open Cascade used by flatbello-occt.
 *
 * Conventions:
 * - Every function returns FB_OK (0) or a negative FB_E* status; fb_last_error() describes the
 *   most recent failure on the calling thread.
 * - No C++ exception crosses this boundary.
 * - Handles are opaque. Indices are 0-based. Faces are numbered in TopExp::MapShapes order;
 *   wires and edges in the order BRepTools_WireExplorer visits them.
 * - Variable-size outputs use two calls: pass NULL (or a too-small capacity) to learn the size,
 *   then call again with a caller-owned buffer.
 * - Lengths are millimetres. Points and vectors are 3 consecutive doubles.
 */
#ifndef FLATBELLO_OCCT_H
#define FLATBELLO_OCCT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum {
    FB_OK = 0,
    FB_EINVAL = -1,    /* bad argument or index */
    FB_EREAD = -2,     /* the file could not be read or translated */
    FB_EEMPTY = -3,    /* the file contains no shape */
    FB_EOCCT = -4,     /* OCCT raised an exception */
    FB_ESIZE = -5,     /* buffer too small; the required size was written */
    FB_EUNKNOWN = -6   /* any other failure */
};

enum { FB_SURFACE_PLANE = 0, FB_SURFACE_CYLINDER = 1, FB_SURFACE_OTHER = 2 };

enum {
    FB_CURVE_LINE = 0,
    FB_CURVE_CIRCLE = 1,
    FB_CURVE_ELLIPSE = 2,
    FB_CURVE_BSPLINE = 3,
    FB_CURVE_OTHER = 4
};

typedef struct fb_shape fb_shape;

/* "major.minor.maintenance" of the OCCT the shim was built against. */
const char *fb_occt_version(void);

/* The message for the last failure on this thread, or "" if none. Valid until the next call. */
const char *fb_last_error(void);

int fb_load_step(const uint8_t *data, size_t len, fb_shape **out);
int fb_load_iges(const uint8_t *data, size_t len, fb_shape **out);
void fb_shape_free(fb_shape *shape);

/* Millimetres per unit of the length unit the file declares (1 if it declares none). */
int fb_shape_file_unit(const fb_shape *shape, double *mm_per_unit);

int fb_face_count(fb_shape *shape, size_t *count);

typedef struct {
    int surface;        /* FB_SURFACE_* */
    int reversed;       /* the face is reversed relative to its surface */
    double origin[3];   /* planes only */
    double normal[3];   /* planes only: outward normal, orientation already applied */
    double x_dir[3];    /* planes only */
    double area;        /* mm^2 */
    size_t wire_count;
} fb_face_info;

int fb_face_info_get(fb_shape *shape, size_t face, fb_face_info *out);

typedef struct {
    int is_outer;
    size_t edge_count;
} fb_wire_info;

int fb_wire_info_get(fb_shape *shape, size_t face, size_t wire, fb_wire_info *out);

typedef struct {
    int curve;          /* FB_CURVE_* */
    int reversed;       /* the wire runs the curve from last to first */
    int degenerated;    /* a zero-length edge; skip it */
    double first, last; /* parameter range */
    double start[3];    /* curve point at `first` */
    double end[3];      /* curve point at `last` */
    double center[3];   /* circle, ellipse */
    double axis[3];     /* circle, ellipse */
    double x_dir[3];    /* circle, ellipse */
    double radius1;     /* circle radius or ellipse major radius */
    double radius2;     /* ellipse minor radius */
    size_t degree;      /* B-spline, trimmed to [first, last] */
    size_t pole_count;
    size_t knot_count;  /* expanded by multiplicity */
    int rational;
} fb_edge_info;

int fb_edge_info_get(fb_shape *shape, size_t face, size_t wire, size_t edge, fb_edge_info *out);

/* The trimmed B-spline of a FB_CURVE_BSPLINE edge. Sizes come from fb_edge_info_get.
 * `weights` may be NULL for a non-rational curve. */
int fb_edge_bspline(fb_shape *shape, size_t face, size_t wire, size_t edge, double *poles,
                    size_t pole_cap, double *weights, double *knots, size_t knot_cap);

/* Points along any edge from `first` to `last`, within `deflection` mm. `points` holds
 * 3 * cap doubles; `count` receives the number of points. */
int fb_edge_sample(fb_shape *shape, size_t face, size_t wire, size_t edge, double deflection,
                   double *points, size_t cap, size_t *count);

/* Meshes every face with the given linear deflection (mm). Call before fb_face_mesh. */
int fb_tessellate(fb_shape *shape, double deflection);

/* A face's triangles, wound counter-clockwise seen from outside. `nodes` holds 3 * node_cap
 * doubles, `triangles` 3 * tri_cap indices into the nodes. A face without a mesh has 0 of each. */
int fb_face_mesh(fb_shape *shape, size_t face, double *nodes, size_t node_cap, size_t *node_count,
                 uint32_t *triangles, size_t tri_cap, size_t *tri_count);

#ifdef FB_FIXTURES
/* Fixture-only construction, for tools/gen-fixtures. */
int fb_make_box(double x, double y, double z, double dx, double dy, double dz, fb_shape **out);
int fb_make_cylinder(const double base[3], const double axis[3], double radius, double height,
                     fb_shape **out);
/* A prism of `height` along +z over the closed planar curve: a B-spline of the given degree
 * through `poles` (n points, clamped uniform knots), closed back to its start by a line. */
int fb_make_bspline_prism(const double *poles, size_t n, size_t degree, double height,
                          fb_shape **out);
int fb_fuse(const fb_shape *a, const fb_shape *b, fb_shape **out);
int fb_cut(const fb_shape *a, const fb_shape *b, fb_shape **out);
int fb_write_step(const fb_shape *shape, const char *path);
int fb_write_iges(const fb_shape *shape, const char *path);
/* Meshes with `deflection` mm and writes STL, binary unless `ascii` is non-zero. */
int fb_write_stl(fb_shape *shape, const char *path, double deflection, int ascii);
#endif

#ifdef __cplusplus
}
#endif

#endif
