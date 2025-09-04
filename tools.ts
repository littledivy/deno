const tools = [
  {
    name: "encode_base64",
    desc: "Encode text to base64",
    input_schema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "Text to encode in base64",
        },
      },
      required: ["text"],
    },
    fn: (input: { text: string }) => {
      const encoded = btoa(input.text);
      return {
        original: input.text,
        encoded,
        length: encoded.length,
      };
    },
  },
  {
    name: "simple_tool",
    desc: "A tool without custom input schema (uses default empty schema)",
    fn: (input: any) => {
      return {
        message: "This tool doesn't require specific inputs",
        received: input,
      };
    },
  },
];

globalThis.tools = tools; // export default tool;
