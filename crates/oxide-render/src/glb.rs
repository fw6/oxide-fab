//! Loading the GLB the Blender export wrote.
//!
//! Node transforms are baked into the vertices at load: the scene graph
//! positions parts with a single translation (the explosion offset), so a mesh
//! with its world transform already applied is all the renderer needs. The
//! manifest's node names are the keys, and `validate` refuses a GLB that does
//! not contain every node the manifest promises.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use oxide_scene::FabManifest;

use crate::RenderError;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Base colour factor from the glTF material, sRGB.
    pub base_color: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    meshes: BTreeMap<String, Mesh>,
    nodes: Vec<String>,
}

impl Model {
    /// Parse a GLB and bake every node's world transform into its mesh.
    pub fn load(bytes: &[u8]) -> Result<Self, RenderError> {
        let (document, buffers, _images) =
            gltf::import_slice(bytes).map_err(|error| RenderError::Glb(error.to_string()))?;
        let mut model = Self::default();
        let mut walked = false;
        for scene in document.scenes() {
            for root in scene.nodes() {
                walked = true;
                model.walk(&root, Mat4::IDENTITY, &buffers);
            }
        }
        // A GLB without scenes still has nodes worth loading.
        if !walked {
            for node in document.nodes() {
                model.walk(&node, Mat4::IDENTITY, &buffers);
            }
        }
        Ok(model)
    }

    fn walk(&mut self, node: &gltf::Node<'_>, parent: Mat4, buffers: &[gltf::buffer::Data]) {
        let name = node.name().unwrap_or("").to_owned();
        let local = Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;
        if !name.is_empty() {
            self.nodes.push(name.clone());
        }
        if let Some(mesh) = node.mesh() {
            let mut combined = Mesh {
                vertices: Vec::new(),
                indices: Vec::new(),
                base_color: [0.8, 0.82, 0.86, 1.0],
            };
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .map_or_else(Vec::new, Iterator::collect);
                if positions.is_empty() {
                    continue;
                }
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .map_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()], Iterator::collect);
                let normal_matrix = world.inverse().transpose();
                let base = combined.vertices.len() as u32;
                for (position, normal) in positions.iter().zip(normals.iter()) {
                    let point = world.transform_point3(Vec3::from(*position));
                    let n = normal_matrix
                        .transform_vector3(Vec3::from(*normal))
                        .normalize_or_zero();
                    combined.vertices.push(Vertex {
                        position: point.to_array(),
                        normal: n.to_array(),
                    });
                }
                let indices: Vec<u32> = reader.read_indices().map_or_else(
                    || (0..positions.len() as u32).collect(),
                    |indices| indices.into_u32().collect(),
                );
                combined
                    .indices
                    .extend(indices.iter().map(|index| index + base));
                let factor = primitive
                    .material()
                    .pbr_metallic_roughness()
                    .base_color_factor();
                combined.base_color = factor;
            }
            if !combined.vertices.is_empty() && !name.is_empty() {
                self.meshes.insert(name.clone(), combined);
            }
        }
        for child in node.children() {
            self.walk(&child, world, buffers);
        }
    }

    #[must_use]
    pub fn mesh(&self, node: &str) -> Option<&Mesh> {
        self.meshes.get(node)
    }

    pub fn meshes(&self) -> impl Iterator<Item = (&String, &Mesh)> {
        self.meshes.iter()
    }

    #[must_use]
    pub fn node_names(&self) -> &[String] {
        &self.nodes
    }

    #[must_use]
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.meshes.values().map(|mesh| mesh.vertices.len()).sum()
    }

    /// Every node the manifest names must have geometry in the GLB.
    pub fn validate(&self, manifest: &FabManifest) -> Result<(), RenderError> {
        let missing: Vec<&str> = manifest
            .expected_nodes()
            .into_iter()
            .filter(|node| !self.meshes.contains_key(*node))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(RenderError::MissingNodes(missing.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLB: &[u8] = include_bytes!("../../../assets/fab/oxide-fab.glb");
    const MANIFEST: &str = include_str!("../../../assets/fab/fab-manifest.json");

    #[test]
    fn every_manifest_node_is_in_the_committed_glb() {
        let manifest = FabManifest::from_json(MANIFEST).unwrap();
        let model = Model::load(GLB).unwrap();
        model.validate(&manifest).unwrap();
        assert_eq!(model.mesh_count(), manifest.expected_nodes().len());
        assert!(model.vertex_count() > 0);
    }

    #[test]
    fn the_tool_parts_are_baked_at_their_manifest_boxes() {
        let manifest = FabManifest::from_json(MANIFEST).unwrap();
        let model = Model::load(GLB).unwrap();
        let part = &manifest.tool("CLN-01").unwrap().parts[0];
        let mesh = model.mesh(&part.node).unwrap();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for vertex in &mesh.vertices {
            let point = Vec3::from(vertex.position);
            lo = lo.min(point);
            hi = hi.max(point);
        }
        for axis in 0..3 {
            assert!((lo[axis] - part.aabb.min[axis]).abs() < 0.02, "min {axis}");
            assert!((hi[axis] - part.aabb.max[axis]).abs() < 0.02, "max {axis}");
        }
    }
}
